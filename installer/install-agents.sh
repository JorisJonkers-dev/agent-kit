#!/usr/bin/env bash
# Full client agents-system installer for Claude Code and Codex.
#
# This is a thin WRAPPER around the base knowledge-system installer
# (installer/install.sh). It:
#   1. Delegates the base install (hooks + skills + council + speckit)
#      by fetching and running the sibling install.sh from the same KB.
#   2. Registers the `knowledge` MCP server with each agent (the NEW
#      capability this installer adds over the base installer).
#   3. Checks council runtime prerequisites (python3 and Node >=22) and
#      best-effort installs the kit's Python dependencies.
#
# It installs for ALL agents (claude + codex) by default. Use
# `--scope user|project` to choose the client homes to manage, `--dry-run`
# to preview, and `--uninstall` to remove everything (base files + the
# knowledge MCP entry).
#
# Released versions are pinned by the @VERSION@ token below, which
# the knowledge-api templates at request time from `SERVICE_VERSION`
# (or `unknown` for local builds).

set -euo pipefail

readonly INSTALLER_VERSION='@VERSION@'
readonly KB_URL='@KB_URL@'

DRY_RUN=0
UNINSTALL=0
SCOPE=user
INCLUDE_OPTIONAL=1

usage() {
  cat <<USAGE
agent-kit full agents-system installer ${INSTALLER_VERSION}

Installs the full client agents system for BOTH Claude Code and Codex:
hooks + skills + council + Spec Kit (delegated to the base installer), the
Claude hook wiring, the kit's Python dependencies, and the workstation MCP
fleet: knowledge + context7 + vuetify (HTTP), plus playwright and serena
(stdio via npx/uvx when present). Pairs with the knowledge-api at ${KB_URL}.
The runner-only servers github (gh-mcp-wrapper) and kubernetes (in-cluster)
are intentionally not installed on a workstation.

Usage:
  curl -fsSL -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
    ${KB_URL}/install-agents.sh | bash [-s -- [--scope user|project] [--no-optional] [--dry-run|--uninstall]]

Options:
  --scope       Install scope. "user" writes to client config homes; "project"
                writes to .claude/.codex under AGENT_KIT_PROJECT_ROOT or \$PWD.
                Defaults to "user".
  --no-optional Skip the stdio MCP servers that need extra tooling
                (playwright via npx, serena via uvx). Install only the HTTP fleet.
  --dry-run     Print every change without modifying the filesystem.
  --uninstall   Remove the delegated base files, the MCP fleet, and Claude hooks.
  --help        Show this help and exit.

Environment:
  KB_BEARER_TOKEN     Required. Bearer token for the knowledge-api.
  CLAUDE_CONFIG_DIR   Override the Claude Code config root (default ~/.claude).
  CODEX_HOME          Override the Codex config root (default ~/.codex).
  AGENT_KIT_PROJECT_ROOT
                      Project root for --scope project (default current directory).
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      shift
      [ "$#" -gt 0 ] || { echo "--scope requires user or project" >&2; exit 64; }
      SCOPE="$1"
      ;;
    --scope=*) SCOPE="${1#--scope=}" ;;
    --dry-run) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --no-optional) INCLUDE_OPTIONAL=0 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 64 ;;
  esac
  shift
done

case "${SCOPE}" in
  user|project) ;;
  *)
    echo "--scope must be user or project" >&2
    usage >&2
    exit 64
    ;;
esac

log() { printf 'agent-kit full installer: %s\n' "$*"; }

if [ -z "${KB_BEARER_TOKEN:-}" ]; then
  echo "agent-kit full installer: KB_BEARER_TOKEN is required in the environment" >&2
  echo "  export KB_BEARER_TOKEN=\"<your-token>\"" >&2
  exit 64
fi

if [ "${SCOPE}" = "project" ]; then
  PROJECT_ROOT="${AGENT_KIT_PROJECT_ROOT:-$PWD}"
  [ -d "${PROJECT_ROOT}" ] || { echo "project root does not exist: ${PROJECT_ROOT}" >&2; exit 64; }
  PROJECT_ROOT="$(cd "${PROJECT_ROOT}" && pwd)"
  readonly PROJECT_ROOT
  readonly CLAUDE_HOME="${PROJECT_ROOT}/.claude"
  readonly CODEX_CONFIG_HOME="${PROJECT_ROOT}/.codex"
  readonly CLAUDE_MCP_FILE="${PROJECT_ROOT}/.mcp.json"
else
  readonly CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  readonly CODEX_CONFIG_HOME="${CODEX_HOME:-$HOME/.codex}"
  # The Claude USER config file is ~/.claude.json (top-level mcpServers key),
  # not a file under the config dir.
  readonly CLAUDE_MCP_FILE="${HOME}/.claude.json"
fi
readonly CODEX_CONFIG_FILE="${CODEX_CONFIG_HOME}/config.toml"
readonly CLAUDE_HOOKS_DIR="${CLAUDE_HOME}/hooks"
readonly CLAUDE_SETTINGS_FILE="${CLAUDE_HOME}/settings.json"
readonly KB_MCP_URL="${KB_URL%/}/mcp"

# -----------------------------------------------------------------
# Step 1: delegate the base install (hooks/skills/council/speckit)
# by fetching and running the sibling base installer from the KB.
# -----------------------------------------------------------------
delegate_base_install() {
  local args=(--agent all --scope "${SCOPE}")
  if [ "${DRY_RUN}" = 1 ]; then args+=(--dry-run); fi
  if [ "${UNINSTALL}" = 1 ]; then args+=(--uninstall); fi
  log "delegating base install: ${KB_URL%/}/install.sh ${args[*]}"
  if ! curl -fsSL -H "Authorization: Bearer ${KB_BEARER_TOKEN}" "${KB_URL%/}/install.sh" \
    | bash -s -- "${args[@]}"; then
    echo "agent-kit full installer: base install failed (curl/bash of install.sh)" >&2
    exit 1
  fi
}

# -----------------------------------------------------------------
# Step 2: register (or remove) the portable MCP fleet for Claude and
# Codex. The runner full-diagnostic profile has 7 servers; this
# installs the workstation-portable subset:
#   - knowledge  (HTTP, our KB, bearer)
#   - context7   (HTTP, public docs)
#   - vuetify    (HTTP, public docs)
#   - playwright (stdio via npx)   — optional, needs node/npx
#   - serena     (stdio via uvx)   — optional, needs uv/uvx
# The runner-only servers are intentionally NOT installed locally:
# github (gh-mcp-wrapper mints in-cluster App tokens) and kubernetes
# (an in-cluster service URL unreachable from a workstation). Use the
# fleet env knobs to opt out:
#   --no-optional  skip the npx/uvx stdio servers entirely.
# These hooks register/remove a fixed set of MANAGED server names, so
# re-runs are idempotent and unrelated hand-added servers are kept.
# -----------------------------------------------------------------
fleet_have() { command -v "$1" >/dev/null 2>&1; }

# Whether each optional stdio server qualifies (opted in AND launcher present).
fleet_optional_state() {
  AK_PLAYWRIGHT=0
  AK_SERENA=0
  if [ "${INCLUDE_OPTIONAL}" = 1 ]; then
    if fleet_have npx; then AK_PLAYWRIGHT=1; else log "skipping playwright MCP (npx not on PATH)"; fi
    if fleet_have uvx; then AK_SERENA=1; else log "skipping serena MCP (uvx not on PATH)"; fi
  fi
}

register_claude_mcp() {
  fleet_optional_state
  if [ "${DRY_RUN}" = 1 ]; then
    log "would register Claude MCP fleet (knowledge, context7, vuetify$([ "${AK_PLAYWRIGHT}" = 1 ] && printf ', playwright')$([ "${AK_SERENA}" = 1 ] && printf ', serena')) in ${CLAUDE_MCP_FILE}"
    return
  fi
  AK_MODE=install AK_CLAUDE_MCP_FILE="${CLAUDE_MCP_FILE}" AK_KB_URL="${KB_MCP_URL}" \
    AK_KB_BEARER="${KB_BEARER_TOKEN}" AK_PLAYWRIGHT="${AK_PLAYWRIGHT}" AK_SERENA="${AK_SERENA}" \
    python3 - <<'PY'
import json
import os
import pathlib

path = pathlib.Path(os.environ["AK_CLAUDE_MCP_FILE"])
mode = os.environ["AK_MODE"]
managed = ["knowledge", "context7", "vuetify", "playwright", "serena"]

fleet = {
    "knowledge": {
        "type": "http",
        "url": os.environ["AK_KB_URL"],
        "headers": {"Authorization": "Bearer " + os.environ["AK_KB_BEARER"]},
    },
    "context7": {"type": "http", "url": "https://mcp.context7.com/mcp"},
    "vuetify": {"type": "http", "url": "https://mcp.vuetifyjs.com/mcp"},
}
if os.environ.get("AK_PLAYWRIGHT") == "1":
    fleet["playwright"] = {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@playwright/mcp@latest", "--headless", "--browser", "chromium"],
    }
if os.environ.get("AK_SERENA") == "1":
    fleet["serena"] = {
        "type": "stdio",
        "command": "uvx",
        "args": [
            "--from", "git+https://github.com/oraios/serena",
            "serena", "start-mcp-server",
            "--context", "claude-code",
            "--project-from-cwd",
            "--mode", "no-memories",
            "--open-web-dashboard", "false",
        ],
    }

data = {}
if path.exists():
    try:
        data = json.loads(path.read_text() or "{}")
    except json.JSONDecodeError:
        data = {}
if not isinstance(data, dict):
    data = {}
servers = data.get("mcpServers")
if not isinstance(servers, dict):
    servers = {}

for name in managed:
    servers.pop(name, None)
if mode == "install":
    servers.update(fleet)

if servers:
    data["mcpServers"] = servers
else:
    data.pop("mcpServers", None)
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(data, indent=2) + "\n")
PY
  log "registered Claude MCP fleet in ${CLAUDE_MCP_FILE}"
}

remove_claude_mcp() {
  if [ ! -e "${CLAUDE_MCP_FILE}" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove the Claude MCP fleet from ${CLAUDE_MCP_FILE}"
    return
  fi
  AK_MODE=remove AK_CLAUDE_MCP_FILE="${CLAUDE_MCP_FILE}" AK_KB_URL="${KB_MCP_URL}" \
    AK_KB_BEARER="${KB_BEARER_TOKEN}" AK_PLAYWRIGHT=0 AK_SERENA=0 \
    python3 - <<'PY'
import json
import os
import pathlib

path = pathlib.Path(os.environ["AK_CLAUDE_MCP_FILE"])
managed = ["knowledge", "context7", "vuetify", "playwright", "serena"]
try:
    data = json.loads(path.read_text() or "{}")
except (json.JSONDecodeError, FileNotFoundError):
    raise SystemExit(0)
if not isinstance(data, dict):
    raise SystemExit(0)
servers = data.get("mcpServers")
if not isinstance(servers, dict):
    raise SystemExit(0)
for name in managed:
    servers.pop(name, None)
if servers:
    data["mcpServers"] = servers
else:
    data.pop("mcpServers", None)
path.write_text(json.dumps(data, indent=2) + "\n")
PY
  log "removed the Claude MCP fleet from ${CLAUDE_MCP_FILE}"
}

register_codex_mcp() {
  fleet_optional_state
  if [ "${DRY_RUN}" = 1 ]; then
    log "would register Codex MCP fleet (knowledge, context7, vuetify$([ "${AK_PLAYWRIGHT}" = 1 ] && printf ', playwright')$([ "${AK_SERENA}" = 1 ] && printf ', serena')) in ${CODEX_CONFIG_FILE}"
    return
  fi
  mkdir -p "${CODEX_CONFIG_HOME}"
  AK_MODE=install AK_CODEX_CONFIG_FILE="${CODEX_CONFIG_FILE}" AK_KB_URL="${KB_MCP_URL}" \
    AK_PLAYWRIGHT="${AK_PLAYWRIGHT}" AK_SERENA="${AK_SERENA}" python3 - <<'PY'
import os
import pathlib
import re

path = pathlib.Path(os.environ["AK_CODEX_CONFIG_FILE"])
mode = os.environ["AK_MODE"]
managed = ["knowledge", "context7", "vuetify", "playwright", "serena"]


def toml_str(value):
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def toml_arr(values):
    return "[" + ", ".join(toml_str(v) for v in values) + "]"


blocks = []
blocks.append(
    f'[mcp_servers.knowledge]\nurl = {toml_str(os.environ["AK_KB_URL"])}\n'
    'bearer_token_env_var = "KB_BEARER_TOKEN"\n'
)
blocks.append(f'[mcp_servers.context7]\nurl = {toml_str("https://mcp.context7.com/mcp")}\n')
blocks.append(f'[mcp_servers.vuetify]\nurl = {toml_str("https://mcp.vuetifyjs.com/mcp")}\n')
if os.environ.get("AK_PLAYWRIGHT") == "1":
    blocks.append(
        '[mcp_servers.playwright]\ncommand = "npx"\n'
        f'args = {toml_arr(["-y", "@playwright/mcp@latest", "--headless", "--browser", "chromium"])}\n'
    )
if os.environ.get("AK_SERENA") == "1":
    blocks.append(
        '[mcp_servers.serena]\ncommand = "uvx"\nstartup_timeout_sec = 60\n'
        f'args = {toml_arr(["--from", "git+https://github.com/oraios/serena", "serena", "start-mcp-server", "--context=codex", "--project-from-cwd", "--mode", "no-memories", "--open-web-dashboard", "false"])}\n'
    )

existing = path.read_text() if path.exists() else ""
# Strip any managed blocks we previously wrote, leaving everything else intact.
lines = existing.splitlines(keepends=True)
out = []
skip = False
managed_headers = {f"[mcp_servers.{name}]" for name in managed}
for line in lines:
    stripped = line.strip()
    if stripped in managed_headers:
        skip = True
        continue
    if skip:
        if stripped.startswith("["):
            skip = False
        else:
            continue
    out.append(line)
text = "".join(out)
if mode == "install":
    if text and not text.endswith("\n"):
        text += "\n"
    text += "\n" + "\n".join(blocks)
text = re.sub(r"\n{3,}", "\n\n", text)
path.write_text(text.lstrip("\n"))
PY
  log "registered Codex MCP fleet in ${CODEX_CONFIG_FILE}"
}

remove_codex_mcp() {
  if [ ! -e "${CODEX_CONFIG_FILE}" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove the Codex MCP fleet from ${CODEX_CONFIG_FILE}"
    return
  fi
  AK_MODE=remove AK_CODEX_CONFIG_FILE="${CODEX_CONFIG_FILE}" AK_KB_URL="${KB_MCP_URL}" \
    AK_PLAYWRIGHT=0 AK_SERENA=0 python3 - <<'PY'
import os
import pathlib
import re

path = pathlib.Path(os.environ["AK_CODEX_CONFIG_FILE"])
managed = ["knowledge", "context7", "vuetify", "playwright", "serena"]
managed_headers = {f"[mcp_servers.{name}]" for name in managed}
lines = path.read_text().splitlines(keepends=True)
out = []
skip = False
for line in lines:
    stripped = line.strip()
    if stripped in managed_headers:
        skip = True
        continue
    if skip:
        if stripped.startswith("["):
            skip = False
        else:
            continue
    out.append(line)
text = re.sub(r"\n{3,}", "\n\n", "".join(out))
path.write_text(text.lstrip("\n"))
PY
  log "removed the Codex MCP fleet from ${CODEX_CONFIG_FILE}"
}

# -----------------------------------------------------------------
# Step 2b: wire the Claude hooks into settings.json. The base
# installer writes the hook scripts but leaves Claude settings.json
# to the operator (it prints manual steps). The full installer does
# it for them, mirroring how the base installer auto-writes Codex
# hooks.json. Idempotent and content-preserving: only the groups that
# reference our own hook scripts are managed.
# -----------------------------------------------------------------
claude_hooks_merge() {
  AK_MODE="$1" AK_SETTINGS_FILE="${CLAUDE_SETTINGS_FILE}" AK_HOOKS_DIR="${CLAUDE_HOOKS_DIR}" \
    python3 - <<'PY'
import json
import os
import pathlib

mode = os.environ["AK_MODE"]
path = pathlib.Path(os.environ["AK_SETTINGS_FILE"])
hooks_dir = os.environ["AK_HOOKS_DIR"]

owned = {
    "pre-tool-use-edit-recall.sh",
    "pre-tool-use-git-commit-capture.sh",
    "stop-session-digest.sh",
}


def cmd(script):
    return f"{hooks_dir}/{script}"


desired = {
    "PreToolUse": [
        {"matcher": "Edit|Write|MultiEdit", "hooks": [{"type": "command", "command": cmd("pre-tool-use-edit-recall.sh"), "timeout": 5}]},
        {"matcher": "Bash", "hooks": [{"type": "command", "command": cmd("pre-tool-use-git-commit-capture.sh"), "timeout": 5}]},
    ],
    "Stop": [
        {"matcher": ".*", "hooks": [{"type": "command", "command": cmd("stop-session-digest.sh"), "async": True, "timeout": 60}]},
    ],
}

data = {}
if path.exists():
    try:
        data = json.loads(path.read_text() or "{}")
    except json.JSONDecodeError:
        data = {}
if not isinstance(data, dict):
    data = {}
hooks = data.get("hooks")
if not isinstance(hooks, dict):
    hooks = {}


def owns(group):
    for hook in group.get("hooks", []) if isinstance(group, dict) else []:
        command = hook.get("command", "") if isinstance(hook, dict) else ""
        if command.rsplit("/", 1)[-1] in owned:
            return True
    return False


for event in ("PreToolUse", "Stop"):
    existing = [g for g in hooks.get(event, []) if isinstance(g, dict) and not owns(g)]
    if mode == "install":
        existing = desired[event] + existing
    if existing:
        hooks[event] = existing
    else:
        hooks.pop(event, None)

if hooks:
    data["hooks"] = hooks
else:
    data.pop("hooks", None)

path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(data, indent=2) + "\n")
PY
}

register_claude_hooks() {
  if [ "${DRY_RUN}" = 1 ]; then
    log "would register Claude recall/capture/digest hooks in ${CLAUDE_SETTINGS_FILE}"
    return
  fi
  claude_hooks_merge install
  log "registered Claude hooks in ${CLAUDE_SETTINGS_FILE}"
}

remove_claude_hooks() {
  if [ ! -e "${CLAUDE_SETTINGS_FILE}" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove Claude hooks from ${CLAUDE_SETTINGS_FILE}"
    return
  fi
  claude_hooks_merge remove
  log "removed Claude hooks from ${CLAUDE_SETTINGS_FILE}"
}

# -----------------------------------------------------------------
# Step 3: council runtime prerequisites and best-effort Python deps.
# -----------------------------------------------------------------
ensure_node_runtime() {
  if ! command -v node >/dev/null 2>&1; then
    log "WARNING: node is not on PATH; council requires Node >=22 — install node manually"
    return
  fi
  local node_version node_major
  node_version="$(node --version 2>/dev/null || true)"
  node_major="${node_version#v}"
  node_major="${node_major%%.*}"
  case "${node_major}" in
    ""|*[!0-9]*)
      log "WARNING: could not parse node version '${node_version}'; council requires Node >=22"
      return
      ;;
  esac
  if [ "${node_major}" -lt 22 ]; then
    log "WARNING: node ${node_version} is on PATH; council requires Node >=22"
  fi
}

ensure_python_deps() {
  if ! command -v python3 >/dev/null 2>&1; then
    log "WARNING: python3 is not on PATH; council requires it — install python3 manually"
    return
  fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would install Python deps: PyYAML>=6.0,<7 ruff>=0.8,<1 (python3 -m pip install --user)"
    return
  fi
  if ! python3 -m pip --version >/dev/null 2>&1; then
    log "pip is unavailable; skipping Python dependency install (PyYAML, ruff)"
    return
  fi
  if python3 -m pip install --user 'PyYAML>=6.0,<7' 'ruff>=0.8,<1' >/dev/null 2>&1; then
    log "installed Python deps (PyYAML>=6.0,<7, ruff>=0.8,<1)"
  else
    log "best-effort Python dependency install failed; continuing"
  fi
}

# -----------------------------------------------------------------
# Drive the install.
# -----------------------------------------------------------------
if [ "${UNINSTALL}" = 1 ]; then
  log "uninstalling full agents system (${INSTALLER_VERSION}, scope=${SCOPE})"
  delegate_base_install
  remove_claude_mcp
  remove_codex_mcp
  remove_claude_hooks
  log "done"
  exit 0
fi

log "installing full agents system (${INSTALLER_VERSION}, scope=${SCOPE})"
delegate_base_install
register_claude_mcp
register_codex_mcp
register_claude_hooks
ensure_node_runtime
ensure_python_deps
log "done"

cat <<EOF
agent-kit full installer complete (${INSTALLER_VERSION}, scope=${SCOPE}).

Registered the MCP fleet (knowledge, context7, vuetify, and — when npx/uvx are
present — playwright, serena) for Claude and Codex, and wired the Claude
recall/capture/digest hooks into settings.json, on top of the base hooks +
skills + council + Spec Kit install. The runner-only github and kubernetes
servers are not installed on a workstation.

Make sure KB_BEARER_TOKEN is set in each agent's environment:
  export KB_BEARER_TOKEN="<your-token>"

Re-run with --uninstall to remove the base files and the knowledge MCP entry.
EOF
