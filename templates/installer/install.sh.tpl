#!/usr/bin/env bash
# knowledge-system installer for Claude Code and Codex clients.
#
# Writes the local hooks + skills that pair with the knowledge-api
# MCP server. Idempotent: re-running picks up any updates the server
# ships in subsequent versions. Use `--agent claude|codex|all` and
# `--scope user|project` to choose the client homes to manage. Use
# `--dry-run` to preview the changes before they land. Use `--uninstall`
# to remove them.
#
# Released versions are pinned by the @VERSION@ token below, which
# the knowledge-api templates at request time from
# `SERVICE_VERSION` (or `unknown` for local builds).

set -euo pipefail

readonly INSTALLER_VERSION='@VERSION@'
readonly KB_URL='@KB_URL@'

DRY_RUN=0
UNINSTALL=0
AGENT=claude
SCOPE=user

usage() {
  cat <<USAGE
knowledge-system installer ${INSTALLER_VERSION}

Writes Claude Code and/or Codex hooks + skills that pair with the MCP server at
${KB_URL}.

Usage:
  curl -fsSL -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
    ${KB_URL}/install.sh | bash [-s -- [--agent claude|codex|all] [--scope user|project] [--dry-run|--uninstall]]

Options:
  --agent      Client home to manage. Defaults to "claude" for backwards compatibility.
  --scope      Install scope. "user" writes to client config homes; "project"
               writes to .claude/.codex under AGENT_KIT_PROJECT_ROOT or $PWD.
               Defaults to "user".
  --dry-run     Print every change without modifying the filesystem.
  --uninstall   Remove every file this installer would write.
  --help        Show this help and exit.

Environment:
  CLAUDE_CONFIG_DIR   Override the Claude Code config root (default ~/.claude).
  CODEX_HOME          Override the Codex config root (default ~/.codex).
  AGENT_KIT_PROJECT_ROOT
                      Project root for --scope project (default current directory).
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --agent)
      shift
      [ "$#" -gt 0 ] || { echo "--agent requires claude, codex, or all" >&2; exit 64; }
      AGENT="$1"
      ;;
    --agent=*) AGENT="${1#--agent=}" ;;
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

case "${AGENT}" in
  claude)
    INSTALL_CLAUDE=1
    INSTALL_CODEX=0
    ;;
  codex)
    INSTALL_CLAUDE=0
    INSTALL_CODEX=1
    ;;
  all)
    INSTALL_CLAUDE=1
    INSTALL_CODEX=1
    ;;
  *)
    echo "--agent must be claude, codex, or all" >&2
    usage >&2
    exit 64
    ;;
esac

case "${SCOPE}" in
  user|project) ;;
  *)
    echo "--scope must be user or project" >&2
    usage >&2
    exit 64
    ;;
esac

if [ "${SCOPE}" = "project" ]; then
  PROJECT_ROOT="${AGENT_KIT_PROJECT_ROOT:-$PWD}"
  [ -d "${PROJECT_ROOT}" ] || { echo "project root does not exist: ${PROJECT_ROOT}" >&2; exit 64; }
  readonly PROJECT_ROOT
  readonly CLAUDE_HOME="$(cd "${PROJECT_ROOT}" && pwd)/.claude"
  readonly CODEX_HOME="$(cd "${PROJECT_ROOT}" && pwd)/.codex"
else
  readonly CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  readonly CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
fi
readonly HOOKS_DIR="${CLAUDE_HOME}/hooks"
readonly COMMANDS_DIR="${CLAUDE_HOME}/commands"
readonly SKILLS_DIR="${CLAUDE_HOME}/skills"
readonly MANIFEST="${CLAUDE_HOME}/.knowledge-system-version"
readonly CODEX_HOOKS_DIR="${CODEX_HOME}/hooks"
readonly CODEX_SKILLS_DIR="${CODEX_HOME}/skills"
readonly CODEX_HOOKS_CONFIG="${CODEX_HOME}/hooks.json"
readonly CODEX_MANIFEST="${CODEX_HOME}/.knowledge-system-version"
readonly CODEX_ALLOWLIST="${CODEX_HOME}/.knowledge-system-allowlist"

log() { printf 'knowledge-system installer: %s\n' "$*"; }

write_file() {
  local path="$1"
  local mode="$2"
  local content="$3"
  if [ "${DRY_RUN}" = 1 ]; then
    log "would write ${path} (mode ${mode}, $(printf '%s' "$content" | wc -c | tr -d ' ') bytes)"
    return
  fi
  mkdir -p "$(dirname "$path")"
  printf '%s' "$content" > "$path"
  chmod "$mode" "$path"
  log "wrote ${path}"
}

remove_file() {
  local path="$1"
  if [ ! -e "$path" ]; then return; fi
  if [ "${DRY_RUN}" = 1 ]; then
    log "would remove ${path}"
    return
  fi
  rm -f "$path"
  log "removed ${path}"
}

readonly STATE_DIR="${CLAUDE_HOME}/state"
readonly ALLOWLIST="${CLAUDE_HOME}/.knowledge-system-allowlist"

claude_managed_paths=(
  "${HOOKS_DIR}/user-prompt-submit-recall.sh"
  "${HOOKS_DIR}/pre-tool-use-edit-recall.sh"
  "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh"
  "${HOOKS_DIR}/stop-session-digest.sh"
# @agent-kit-speckit-commands-managed
  "${SKILLS_DIR}/topics/SKILL.md"
  "${SKILLS_DIR}/audit/SKILL.md"
  "${SKILLS_DIR}/kb-first/SKILL.md"
  "${SKILLS_DIR}/token-economy/SKILL.md"
  "${SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
# @agent-kit-council-managed claude
  "${ALLOWLIST}"
)

codex_managed_paths=(
  "${CODEX_HOOKS_DIR}/kb-user-prompt-recall.sh"
  "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh"
  "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh"
  "${CODEX_HOOKS_DIR}/kb-stop-digest.sh"
# @agent-kit-codex-speckit-managed
  "${CODEX_SKILLS_DIR}/topics/SKILL.md"
  "${CODEX_SKILLS_DIR}/audit/SKILL.md"
  "${CODEX_SKILLS_DIR}/kb-first/SKILL.md"
  "${CODEX_SKILLS_DIR}/token-economy/SKILL.md"
  "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
# @agent-kit-council-managed codex
  "${CODEX_ALLOWLIST}"
  "${CODEX_HOOKS_CONFIG}"
)

managed_paths=()
if [ "${INSTALL_CLAUDE}" = 1 ]; then
  managed_paths+=("${claude_managed_paths[@]}")
fi
if [ "${INSTALL_CODEX}" = 1 ]; then
  managed_paths+=("${codex_managed_paths[@]}")
fi

if [ "${UNINSTALL}" = 1 ]; then
  log "uninstalling knowledge-system files (${INSTALLER_VERSION}, agent=${AGENT}, scope=${SCOPE})"
  for path in "${managed_paths[@]}"; do
    remove_file "$path"
  done
  if [ "${INSTALL_CLAUDE}" = 1 ]; then
    remove_file "${MANIFEST}"
  fi
  if [ "${INSTALL_CODEX}" = 1 ]; then
    remove_file "${CODEX_MANIFEST}"
  fi
  log "done"
  exit 0
fi

# -----------------------------------------------------------------
# Hook: UserPromptSubmit recall
# -----------------------------------------------------------------
read -r -d '' USER_PROMPT_SUBMIT_HOOK <<'HOOK' || true
# @agent-kit-include partials/hooks/user-prompt-submit-recall.sh
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/user-prompt-submit-recall.sh" 0755 "${USER_PROMPT_SUBMIT_HOOK}"
fi

# -----------------------------------------------------------------
# Spec Kit Claude commands
# -----------------------------------------------------------------
# @agent-kit-speckit-commands-bundle

# -----------------------------------------------------------------
# Skill: topics
# -----------------------------------------------------------------
read -r -d '' TOPICS_SKILL <<'SKILL' || true
# @agent-kit-include partials/skills/topics.md
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/topics/SKILL.md" 0644 "${TOPICS_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: audit
# -----------------------------------------------------------------
read -r -d '' AUDIT_SKILL <<'SKILL' || true
# @agent-kit-include partials/skills/audit.md
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/audit/SKILL.md" 0644 "${AUDIT_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: kb-first
# -----------------------------------------------------------------
read -r -d '' KB_FIRST_SKILL <<'SKILL' || true
# @agent-kit-include partials/skills/kb-first.md
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/kb-first/SKILL.md" 0644 "${KB_FIRST_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: token-economy
# -----------------------------------------------------------------
read -r -d '' TOKEN_ECONOMY_SKILL <<'SKILL' || true
# @agent-kit-include partials/skills/token-economy.md
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/token-economy/SKILL.md" 0644 "${TOKEN_ECONOMY_SKILL}"
fi

# -----------------------------------------------------------------
# Skill: agent-session-bootstrap
# -----------------------------------------------------------------
read -r -d '' AGENT_SESSION_BOOTSTRAP_SKILL <<'SKILL' || true
# @agent-kit-include partials/skills/agent-session-bootstrap.md
SKILL

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${SKILLS_DIR}/agent-session-bootstrap/SKILL.md" 0644 "${AGENT_SESSION_BOOTSTRAP_SKILL}"
fi

# -----------------------------------------------------------------
# Spec Kit Codex skills
# -----------------------------------------------------------------
# @agent-kit-codex-speckit-bundle

# -----------------------------------------------------------------
# Skill: council (multi-file toolkit — driver + prompts + schemas +
# default config). Installs into ${SKILLS_DIR}/council and
# ${CODEX_SKILLS_DIR}/council. council.toml is preserved on upgrade.
# -----------------------------------------------------------------
# @agent-kit-council-bundle

# -----------------------------------------------------------------
# Path allowlist (gitignore-style). Hooks below skip any tool input
# whose target matches a pattern here. Defaults exclude paths that
# typically carry secrets so an Edit on `.env` does not exfiltrate
# the path to the KB recall query.
# -----------------------------------------------------------------
if [ "${INSTALL_CLAUDE}" = 1 ] || [ "${INSTALL_CODEX}" = 1 ]; then
  read -r -d '' ALLOWLIST_DEFAULTS <<'ALLOW' || true
# @agent-kit-include partials/allowlist/defaults.gitignore
ALLOW
fi

if [ "${INSTALL_CLAUDE}" = 1 ] && [ ! -e "${ALLOWLIST}" ]; then
  write_file "${ALLOWLIST}" 0644 "${ALLOWLIST_DEFAULTS}"
elif [ "${INSTALL_CLAUDE}" = 1 ]; then
  log "preserving existing ${ALLOWLIST}"
fi

if [ "${INSTALL_CODEX}" = 1 ] && [ ! -e "${CODEX_ALLOWLIST}" ]; then
  write_file "${CODEX_ALLOWLIST}" 0644 "${ALLOWLIST_DEFAULTS}"
elif [ "${INSTALL_CODEX}" = 1 ]; then
  log "preserving existing ${CODEX_ALLOWLIST}"
fi

# -----------------------------------------------------------------
# Hook: PreToolUse — Edit/Write/MultiEdit/apply_patch recall
# -----------------------------------------------------------------
read -r -d '' PRE_TOOL_USE_EDIT_HOOK <<'HOOK' || true
# @agent-kit-include partials/hooks/pre-tool-use-edit-recall.sh
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/pre-tool-use-edit-recall.sh" 0755 "${PRE_TOOL_USE_EDIT_HOOK}"
fi

# -----------------------------------------------------------------
# Hook: PreToolUse — Bash matching `git commit` capture
# -----------------------------------------------------------------
read -r -d '' PRE_TOOL_USE_GIT_COMMIT_HOOK <<'HOOK' || true
# @agent-kit-include partials/hooks/pre-tool-use-git-commit-capture.sh
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" 0755 "${PRE_TOOL_USE_GIT_COMMIT_HOOK}"
fi

# -----------------------------------------------------------------
# Hook: Stop — session-digest auto-capture
# -----------------------------------------------------------------
read -r -d '' STOP_SESSION_DIGEST_HOOK <<'HOOK' || true
# @agent-kit-include partials/hooks/stop-session-digest-claude.sh
HOOK

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  write_file "${HOOKS_DIR}/stop-session-digest.sh" 0755 "${STOP_SESSION_DIGEST_HOOK}"
fi

# -----------------------------------------------------------------
# Codex project hook mirror
# -----------------------------------------------------------------
read -r -d '' CODEX_STOP_DIGEST_HOOK <<'HOOK' || true
# @agent-kit-include partials/hooks/stop-session-digest-codex.sh
HOOK

read -r -d '' CODEX_HOOKS_JSON <<HOOKS || true
# @agent-kit-include partials/settings/codex-hooks.json
HOOKS

if [ "${INSTALL_CODEX}" = 1 ]; then
  write_file "${CODEX_HOOKS_DIR}/kb-user-prompt-recall.sh" 0755 "${USER_PROMPT_SUBMIT_HOOK}"
  write_file "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh" 0755 "${PRE_TOOL_USE_EDIT_HOOK}"
  write_file "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" 0755 "${PRE_TOOL_USE_GIT_COMMIT_HOOK}"
  write_file "${CODEX_HOOKS_DIR}/kb-stop-digest.sh" 0755 "${CODEX_STOP_DIGEST_HOOK}"
  write_file "${CODEX_SKILLS_DIR}/topics/SKILL.md" 0644 "${TOPICS_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/audit/SKILL.md" 0644 "${AUDIT_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/kb-first/SKILL.md" 0644 "${KB_FIRST_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/token-economy/SKILL.md" 0644 "${TOKEN_ECONOMY_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md" 0644 "${AGENT_SESSION_BOOTSTRAP_SKILL}"
  write_file "${CODEX_HOOKS_CONFIG}" 0644 "${CODEX_HOOKS_JSON}"
fi

# -----------------------------------------------------------------
# Spec Kit project scaffold seed
# -----------------------------------------------------------------
# @agent-kit-specify-seed

# -----------------------------------------------------------------
# Manifest
# -----------------------------------------------------------------
if [ "${DRY_RUN}" != 1 ] && [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat > "${MANIFEST}" <<MANIFEST
# Managed by the knowledge-system installer (${KB_URL}/install.sh).
# Re-run that command to update. Use --uninstall to remove every
# file listed below.
version=${INSTALLER_VERSION}
installed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
scope=${SCOPE}
managed:
$(printf '  - %s\n' "${claude_managed_paths[@]}")
MANIFEST
  log "wrote ${MANIFEST}"
fi

if [ "${DRY_RUN}" != 1 ] && [ "${INSTALL_CODEX}" = 1 ]; then
  cat > "${CODEX_MANIFEST}" <<MANIFEST
# Managed by the knowledge-system installer (${KB_URL}/install.sh).
# Re-run that command to update. Use --agent codex --uninstall to remove every
# Codex file listed below.
version=${INSTALLER_VERSION}
installed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
agent=codex
scope=${SCOPE}
managed:
$(printf '  - %s\n' "${codex_managed_paths[@]}")
MANIFEST
  log "wrote ${CODEX_MANIFEST}"
fi

cat <<EOF
knowledge-system installer complete (${INSTALLER_VERSION}, agent=${AGENT}, scope=${SCOPE}).
EOF

if [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat <<EOF

Claude next steps:

  1. Register the four hooks in ${CLAUDE_HOME}/settings.json under the
     matching "hooks.<event>" arrays. Suggested config:

     "UserPromptSubmit": [
       { "matcher": ".*", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/user-prompt-submit-recall.sh",
           "timeout": 5 } ] } ],
     "PreToolUse": [
       { "matcher": "Edit|Write|MultiEdit", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/pre-tool-use-edit-recall.sh",
           "timeout": 5 } ] },
       { "matcher": "Bash", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/pre-tool-use-git-commit-capture.sh",
           "timeout": 5 } ] } ],
     "Stop": [
       { "matcher": ".*", "hooks": [
         { "type": "command",
           "command": "${HOOKS_DIR}/stop-session-digest.sh",
           "async": true,
           "timeout": 60 } ] } ]

  2. Make sure KB_BEARER_TOKEN is set in the Claude Code environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

if [ "${INSTALL_CODEX}" = 1 ]; then
  cat <<EOF

Codex next steps:

  1. ${CODEX_HOOKS_CONFIG} has been written with UserPromptSubmit, PreToolUse,
     and Stop hooks.
  2. Make sure KB_BEARER_TOKEN is set in the Codex environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

cat <<EOF

Verify with:  curl -sS -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
                     ${KB_URL}/mcp -d '{"jsonrpc":"2.0","id":1,"method":"ping"}'

Safety controls:
  - Panic switch:   export KB_AUTO_MCP_DISABLED=1   (turns every hook into a no-op).
EOF
if [ "${INSTALL_CLAUDE}" = 1 ]; then
  cat <<EOF
  - Claude allowlist: edit ${ALLOWLIST}  (gitignore-style patterns).
  - Claude state:     ${STATE_DIR}/auto-mcp.log + per-session dedupe under ${STATE_DIR}/sessions/.
EOF
fi
if [ "${INSTALL_CODEX}" = 1 ]; then
  cat <<EOF
  - Codex allowlist:  edit ${CODEX_ALLOWLIST}  (gitignore-style patterns).
  - Codex state:      ${CODEX_HOME}/state/auto-mcp.log + per-session dedupe under ${CODEX_HOME}/state/sessions/.
EOF
fi
cat <<EOF
  - Provenance:     every auto-capture lands with source = "<agent>:auto-capture:<hook>"
                    or "claude-code:auto-digest:<session>" so a bulk revoke is one SQL query.

Run with --agent ${AGENT} --scope ${SCOPE} --uninstall to remove every selected file this installer wrote.
EOF
