#!/usr/bin/env bash
# Full client agents-system installer for Claude Code and Codex.
#
# This is a thin WRAPPER around the base knowledge-system installer
# (installer/install.sh). It:
#   1. Delegates the base install (hooks + skills + council + speckit)
#      by fetching and running the sibling install.sh from the same KB.
#   2. Registers the `knowledge` MCP server with each agent (the NEW
#      capability this installer adds over the base installer).
#   3. Best-effort installs the kit's Python dependencies (council needs
#      python3).
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

usage() {
  cat <<USAGE
agent-kit full agents-system installer ${INSTALLER_VERSION}

Installs the full client agents system for BOTH Claude Code and Codex:
hooks + skills + council + Spec Kit (delegated to the base installer), the
\`knowledge\` MCP server registration, and the kit's Python dependencies.
Pairs with the knowledge-api MCP server at ${KB_URL}.

Usage:
  curl -fsSL -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
    ${KB_URL}/install-agents.sh | bash [-s -- [--scope user|project] [--dry-run|--uninstall]]

Options:
  --scope       Install scope. "user" writes to client config homes; "project"
                writes to .claude/.codex under AGENT_KIT_PROJECT_ROOT or \$PWD.
                Defaults to "user".
  --dry-run     Print every change without modifying the filesystem.
  --uninstall   Remove the delegated base files and the knowledge MCP entry.
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
# Step 2: register (or remove) the knowledge MCP server.
# -----------------------------------------------------------------
register_claude_mcp() {
  if command -v claude >/dev/null 2>&1; then
    if [ "${DRY_RUN}" = 1 ]; then
      log "would run: claude mcp add --scope ${SCOPE} --transport http knowledge ${KB_MCP_URL}"
      return
    fi
    # Replace any existing entry so re-runs stay idempotent.
    claude mcp remove --scope "${SCOPE}" knowledge >/dev/null 2>&1 || true
    claude mcp add --scope "${SCOPE}" --transport http knowledge "${KB_MCP_URL}" \
      --header "Authorization: Bearer ${KB_BEARER_TOKEN}"
    log "registered knowledge MCP via claude CLI (scope ${SCOPE})"
    return
  fi
  log "claude CLI not found; merging knowledge MCP into ${CLAUDE_MCP_FILE}"
  if [ "${DRY_RUN}" = 1 ]; then
    log "would merge knowledge MCP into ${CLAUDE_MCP_FILE}"
    return
  fi
  AK_MCP_URL="${KB_MCP_URL}" AK_BEARER="${KB_BEARER_TOKEN}" \
    AK_CLAUDE_MCP_FILE="${CLAUDE_MCP_FILE}" python3 - <<'PY'
import json
import os
import pathlib

path = pathlib.Path(os.environ["AK_CLAUDE_MCP_FILE"])
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
servers["knowledge"] = {
    "type": "http",
    "url": os.environ["AK_MCP_URL"],
    "headers": {"Authorization": "Bearer " + os.environ["AK_BEARER"]},
}
data["mcpServers"] = servers
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(data, indent=2) + "\n")
PY
  log "merged knowledge MCP into ${CLAUDE_MCP_FILE}"
}

remove_claude_mcp() {
  if command -v claude >/dev/null 2>&1; then
    if [ "${DRY_RUN}" = 1 ]; then
      log "would run: claude mcp remove --scope ${SCOPE} knowledge"
      return
    fi
    claude mcp remove --scope "${SCOPE}" knowledge >/dev/null 2>&1 || true
    log "removed knowledge MCP via claude CLI (scope ${SCOPE})"
    return
  fi
  if [ ! -e "${CLAUDE_MCP_FILE}" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove knowledge MCP from ${CLAUDE_MCP_FILE}"
    return
  fi
  AK_CLAUDE_MCP_FILE="${CLAUDE_MCP_FILE}" python3 - <<'PY'
import json
import os
import pathlib

path = pathlib.Path(os.environ["AK_CLAUDE_MCP_FILE"])
try:
    data = json.loads(path.read_text() or "{}")
except (json.JSONDecodeError, FileNotFoundError):
    raise SystemExit(0)
if not isinstance(data, dict):
    raise SystemExit(0)
servers = data.get("mcpServers")
if isinstance(servers, dict) and "knowledge" in servers:
    del servers["knowledge"]
    data["mcpServers"] = servers
    path.write_text(json.dumps(data, indent=2) + "\n")
PY
  log "removed knowledge MCP from ${CLAUDE_MCP_FILE}"
}

register_codex_mcp() {
  if [ "${DRY_RUN}" = 1 ]; then
    log "would ensure [mcp_servers.knowledge] in ${CODEX_CONFIG_FILE}"
    return
  fi
  if [ -e "${CODEX_CONFIG_FILE}" ] && grep -q '^\[mcp_servers\.knowledge\]' "${CODEX_CONFIG_FILE}"; then
    log "preserving existing [mcp_servers.knowledge] in ${CODEX_CONFIG_FILE}"
    return
  fi
  mkdir -p "${CODEX_CONFIG_HOME}"
  cat >> "${CODEX_CONFIG_FILE}" <<TOML

[mcp_servers.knowledge]
url = "${KB_MCP_URL}"
bearer_token_env_var = "KB_BEARER_TOKEN"
TOML
  log "registered [mcp_servers.knowledge] in ${CODEX_CONFIG_FILE}"
}

remove_codex_mcp() {
  if [ ! -e "${CODEX_CONFIG_FILE}" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove [mcp_servers.knowledge] from ${CODEX_CONFIG_FILE}"
    return
  fi
  if ! grep -q '^\[mcp_servers\.knowledge\]' "${CODEX_CONFIG_FILE}"; then return; fi
  AK_CODEX_CONFIG_FILE="${CODEX_CONFIG_FILE}" python3 - <<'PY'
import os
import pathlib
import re

path = pathlib.Path(os.environ["AK_CODEX_CONFIG_FILE"])
lines = path.read_text().splitlines(keepends=True)
out = []
skip = False
for line in lines:
    stripped = line.strip()
    if stripped == "[mcp_servers.knowledge]":
        skip = True
        continue
    if skip:
        # End of block on the next table header.
        if re.match(r"^\[", stripped):
            skip = False
        else:
            continue
    out.append(line)
text = "".join(out)
# Collapse any leading/trailing blank-line churn from the removal.
text = re.sub(r"\n{3,}", "\n\n", text)
path.write_text(text.lstrip("\n"))
PY
  log "removed [mcp_servers.knowledge] from ${CODEX_CONFIG_FILE}"
}

# -----------------------------------------------------------------
# Step 3: best-effort Python deps for council.
# -----------------------------------------------------------------
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
  log "done"
  exit 0
fi

log "installing full agents system (${INSTALLER_VERSION}, scope=${SCOPE})"
delegate_base_install
register_claude_mcp
register_codex_mcp
ensure_python_deps
log "done"

cat <<EOF
agent-kit full installer complete (${INSTALLER_VERSION}, scope=${SCOPE}).

Registered the knowledge MCP server (${KB_MCP_URL}) for Claude and Codex on top
of the base hooks + skills + council + Spec Kit install.

Make sure KB_BEARER_TOKEN is set in each agent's environment:
  export KB_BEARER_TOKEN="<your-token>"

Re-run with --uninstall to remove the base files and the knowledge MCP entry.
EOF
