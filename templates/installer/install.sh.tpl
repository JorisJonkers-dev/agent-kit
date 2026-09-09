#!/usr/bin/env bash
# knowledge-system installer for Claude Code and Codex clients.
#
# Writes the local skills that pair with the knowledge-api
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

Writes Claude Code and/or Codex skills that pair with the MCP server at
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
readonly AGENTS_DIR="${CLAUDE_HOME}/agents"
readonly COMMANDS_DIR="${CLAUDE_HOME}/commands"
readonly SKILLS_DIR="${CLAUDE_HOME}/skills"
readonly MANIFEST="${CLAUDE_HOME}/.knowledge-system-version"
readonly CODEX_SKILLS_DIR="${CODEX_HOME}/skills"
readonly CODEX_MANIFEST="${CODEX_HOME}/.knowledge-system-version"

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


claude_agent_managed_paths=(
)

claude_managed_paths=(
  "${claude_agent_managed_paths[@]}"
# @agent-kit-speckit-commands-managed
  "${SKILLS_DIR}/topics/SKILL.md"
  "${SKILLS_DIR}/audit/SKILL.md"
  "${SKILLS_DIR}/kb-first/SKILL.md"
  "${SKILLS_DIR}/token-economy/SKILL.md"
  "${SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
  "${SKILLS_DIR}/grill-me/SKILL.md"
# @agent-kit-council-managed claude
)

codex_managed_paths=(
# @agent-kit-codex-speckit-managed
  "${CODEX_SKILLS_DIR}/topics/SKILL.md"
  "${CODEX_SKILLS_DIR}/audit/SKILL.md"
  "${CODEX_SKILLS_DIR}/kb-first/SKILL.md"
  "${CODEX_SKILLS_DIR}/token-economy/SKILL.md"
  "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md"
  "${CODEX_SKILLS_DIR}/grill-me/SKILL.md"
# @agent-kit-council-managed codex
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
# Skill: council (multi-file toolkit — launcher + prompts + schemas +
# default config). Installs into ${SKILLS_DIR}/council and
# ${CODEX_SKILLS_DIR}/council. council.toml is preserved on upgrade.
# -----------------------------------------------------------------
# @agent-kit-council-bundle

# -----------------------------------------------------------------
# Third-party skill: grill-me (mattpocock/skills). Installed for both
# CLIs via the `skills` CLI, pinned to a reviewed ref so bumping the
# pin is a normal renovate/review event (org supply-chain posture).
# The skill content executes inside agent context, so the ref is never
# floating. Loud-skip philosophy: if npx or the network is unavailable
# the step logs a warning and continues rather than failing the whole
# install (same fallback posture as the MCP fleet registration). The
# `skills` CLI is itself idempotent, so re-runs and upgrades converge
# without duplicating the skill.
# -----------------------------------------------------------------
# Pin — reviewed 2026-07-13; see skills-source.lock and manifest.yaml
# provenance.skills. mattpocock/skills v1.1.0 = commit
# d574778f94cf620fcc8ce741584093bc650a61d3. Bump via renovate/review.
readonly GRILL_ME_SOURCE='mattpocock/skills#v1.1.0'
readonly GRILL_ME_SKILL='grill-me'
readonly SKILLS_CLI='skills@1.5.16'

install_grill_me() {
  # ${agent} is the `skills` CLI agent name (its registry keys are
  # "claude-code" and "codex", not "claude"); ${home} is the installer's
  # resolved client home for that agent.
  local agent="$1" home="$2"
  if [ "${DRY_RUN}" = 1 ]; then
    log "would install ${GRILL_ME_SKILL} for ${agent} from ${GRILL_ME_SOURCE} via npx ${SKILLS_CLI}"
    return 0
  fi
  if ! command -v npx >/dev/null 2>&1; then
    log "WARNING: npx is not on PATH; skipping ${GRILL_ME_SKILL} for ${agent} (install Node >=22 and re-run)"
    return 0
  fi
  # The `skills` CLI resolves its home from CLAUDE_CONFIG_DIR / CODEX_HOME,
  # so exporting the installer-resolved home lands grill-me next to the
  # kit's own skills for both user and project scope.
  if CLAUDE_CONFIG_DIR="${home}" CODEX_HOME="${home}" \
      npx --yes "${SKILLS_CLI}" add "${GRILL_ME_SOURCE}" \
        --skill="${GRILL_ME_SKILL}" --agent "${agent}" --yes --global; then
    log "installed ${GRILL_ME_SKILL} for ${agent} from ${GRILL_ME_SOURCE}"
  else
    log "WARNING: ${GRILL_ME_SKILL} install for ${agent} failed (npx/network unavailable?); continuing"
  fi
}

# The `skills` CLI expects "claude-code" (not "claude") as the Claude agent
# name; the installer's own INSTALL_CLAUDE gating and ${CLAUDE_HOME}/${SKILLS_DIR}
# variable names are unchanged.
if [ "${INSTALL_CLAUDE}" = 1 ]; then install_grill_me claude-code "${CLAUDE_HOME}"; fi
if [ "${INSTALL_CODEX}" = 1 ]; then install_grill_me codex "${CODEX_HOME}"; fi

if [ "${INSTALL_CODEX}" = 1 ]; then
  write_file "${CODEX_SKILLS_DIR}/topics/SKILL.md" 0644 "${TOPICS_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/audit/SKILL.md" 0644 "${AUDIT_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/kb-first/SKILL.md" 0644 "${KB_FIRST_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/token-economy/SKILL.md" 0644 "${TOKEN_ECONOMY_SKILL}"
  write_file "${CODEX_SKILLS_DIR}/agent-session-bootstrap/SKILL.md" 0644 "${AGENT_SESSION_BOOTSTRAP_SKILL}"
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

  1. Make sure KB_BEARER_TOKEN is set in the Claude Code environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

if [ "${INSTALL_CODEX}" = 1 ]; then
  cat <<EOF

Codex next steps:

  1. Make sure KB_BEARER_TOKEN is set in the Codex environment:
       export KB_BEARER_TOKEN="<your-token>"
EOF
fi

cat <<EOF

Verify with:  curl -sS -H "Authorization: Bearer \$KB_BEARER_TOKEN" \\
                     ${KB_URL}/mcp -d '{"jsonrpc":"2.0","id":1,"method":"ping"}'

Run with --agent ${AGENT} --scope ${SCOPE} --uninstall to remove every selected file this installer wrote.
EOF
