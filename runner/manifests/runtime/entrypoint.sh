#!/bin/sh
set -eu

CODEX_HOME="${CODEX_HOME:-${HOME}/.codex}"
CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-${HOME}/.claude}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-$PWD}"
AGENT_KIT_VERSION_MARKER="${AGENT_KIT_VERSION_MARKER:-.knowledge-system-version}"

check_agent_kit_manifest() {
  agent_name="$1"
  manifest_path="$2"
  expected_version="${AGENT_KIT_EXPECTED_VERSION:-}"

  if [ ! -f "$manifest_path" ]; then
    echo "[entrypoint] WARN: agent kit manifest missing for ${agent_name} at ${manifest_path}; run the agent-kit installer"
    return
  fi

  installed_version=$(awk -F= '/^version=/{print $2; exit}' "$manifest_path" 2>/dev/null || true)
  if [ -z "$installed_version" ]; then
    echo "[entrypoint] WARN: agent kit manifest for ${agent_name} has no version at ${manifest_path}"
    return
  fi

  if [ -n "$expected_version" ] && [ "$installed_version" != "$expected_version" ]; then
    echo "[entrypoint] WARN: agent kit manifest for ${agent_name} is ${installed_version}, expected ${expected_version}"
  fi
}

check_agent_kit_manifests() {
  check_agent_kit_manifest "Claude" "${CLAUDE_CONFIG_DIR}/${AGENT_KIT_VERSION_MARKER}"
  check_agent_kit_manifest "Codex" "${CODEX_HOME}/${AGENT_KIT_VERSION_MARKER}"
}

strip_git_host() {
  value="$1"
  host="${AGENT_GIT_HOST:-}"
  if [ -n "$host" ]; then
    for scheme in ${AGENT_GIT_SCHEMES:-https http}; do
      prefix="${scheme}://${host}/"
      case "$value" in
        "$prefix"*) value="${value#"$prefix"}" ;;
      esac
    done
    value="${value#git@${host}:}"
    value="${value#ssh://git@${host}/}"
  fi
  value="${value%.git}"
  printf '%s' "$value"
}

repo_slug() {
  strip_git_host "$1"
}

repo_dir_name() {
  basename "${1%%#*}" .git
}

derive_repo_allow() {
  _allow=""
  [ -n "${REPO_URL:-}" ] && _allow="$(repo_slug "$REPO_URL")"
  for _entry in $(printf '%s' "${REPO_URLS:-}" | tr ';\n' '  '); do
    _allow="${_allow} $(repo_slug "${_entry%%#*}")"
  done
  printf '%s' "$_allow" | tr -s ' ' | sed -e 's/^ *//' -e 's/ *$//'
}

# What this entrypoint writes into HOME and CODEX_HOME belongs to the runner
# container: the global git config names the credential helper agent-gh-app,
# which only its image ships, and ~/.claude.json and Codex's config.toml accept
# bypass permissions and trust the workspace. Run anywhere else (a workstation
# shell, CI) it would land in the invoking user's own homes and break every
# push there, so it is written only inside a container: the runtime
# Containerfile sets AGENT_RUNNER_CONTAINER=1, and Docker and Podman leave
# /.dockerenv and /run/.containerenv.
in_runner_container() {
  [ "${AGENT_RUNNER_CONTAINER:-}" = 1 ] || [ -f /.dockerenv ] || [ -f /run/.containerenv ]
}

configure_global_git() {
  command -v git >/dev/null 2>&1 || return 0
  git config --global user.name "${GIT_AUTHOR_NAME:-JorisJonkers Agent}"
  git config --global user.email "${GIT_AUTHOR_EMAIL:-agents@jorisjonkers.dev}"
  git config --global init.defaultBranch "${GIT_DEFAULT_BRANCH:-main}"
  git config --global credential.helper agent-gh-app
  git config --global credential.useHttpPath true
  if ! git config --global --get-all safe.directory 2>/dev/null | grep -Fxq "$WORKSPACE_ROOT"; then
    git config --global --add safe.directory "$WORKSPACE_ROOT"
  fi
  if [ -n "${AGENT_GIT_HOST:-}" ]; then
    git_scheme="${AGENT_GIT_SCHEME:-https}"
    # Both SSH URL spellings must rewrite to the credential-helper scheme.
    # A bare `git config` per value overwrites rather than appends, so the
    # second spelling silently dropped the first; --add keeps both. Reset
    # first so this idempotent boot block does not accumulate duplicates.
    git config --global --unset-all "url.${git_scheme}://${AGENT_GIT_HOST}/.insteadOf" 2>/dev/null || true
    git config --global --add "url.${git_scheme}://${AGENT_GIT_HOST}/.insteadOf" "git@${AGENT_GIT_HOST}:"
    git config --global --add "url.${git_scheme}://${AGENT_GIT_HOST}/.insteadOf" "ssh://git@${AGENT_GIT_HOST}/"
  fi
}

configure_claude_home() {
  [ -f "$HOME/.claude.json" ] || echo '{}' > "$HOME/.claude.json"
  command -v jq >/dev/null 2>&1 || return 0
  claude_tmp=$(mktemp)
  if jq --arg ws "$WORKSPACE_ROOT" '
        (.theme //= "dark")
        | (.hasCompletedOnboarding //= true)
        | (.bypassPermissionsModeAccepted //= true)
        | (.projects //= {})
        | (.projects[$ws] //= {})
        | (.projects[$ws].hasTrustDialogAccepted //= true)
        | (.projects[$ws].hasCompletedProjectOnboarding //= true)
      ' "$HOME/.claude.json" > "$claude_tmp"; then
    mv "$claude_tmp" "$HOME/.claude.json"
  else
    rm -f "$claude_tmp"
  fi
}

register_repo_trust() {
  _dir="$1"
  in_runner_container || return 0
  if command -v git >/dev/null 2>&1 &&
     ! git config --global --get-all safe.directory 2>/dev/null | grep -Fxq "$_dir"; then
    git config --global --add safe.directory "$_dir"
  fi

  if [ -f "$HOME/.claude.json" ] && command -v jq >/dev/null 2>&1; then
    _ctmp=$(mktemp)
    if jq --arg d "$_dir" '
          (.projects //= {})
          | (.projects[$d] //= {})
          | (.projects[$d].hasTrustDialogAccepted //= true)
          | (.projects[$d].hasCompletedProjectOnboarding //= true)
        ' "$HOME/.claude.json" > "$_ctmp"; then
      mv "$_ctmp" "$HOME/.claude.json"
    else
      rm -f "$_ctmp"
    fi
  fi

  if [ -f "$CODEX_HOME/config.toml" ] &&
     ! grep -q "^\[projects\.\"${_dir}\"\]" "$CODEX_HOME/config.toml"; then
    printf '\n[projects."%s"]\ntrust_level = "trusted"\n' "$_dir" >> "$CODEX_HOME/config.toml"
  fi
}

render_template_placeholders() {
  sed -e "s|@CLUSTER_MCP_URL@|${AGENT_CLUSTER_MCP_URL:-}|g" \
      -e "s|@FRONTEND_DOCS_MCP_URL@|${AGENT_FRONTEND_DOCS_MCP_URL:-}|g" \
      -e "s|@UI_DOCS_MCP_URL@|${AGENT_UI_DOCS_MCP_URL:-}|g" \
      "$1"
}

select_profile_file() {
  _prefix="$1"
  _suffix="$2"
  _override="$3"
  _profile="$4"
  _dir="$5"
  if [ -n "$_override" ]; then
    printf '%s' "$_override"
    return
  fi
  _profile_file="${_dir}/${_prefix}.${_profile}.${_suffix}"
  _minimal_file="${_dir}/${_prefix}.minimal.${_suffix}"
  _legacy_file="${_dir}/${_prefix}.${_suffix}"
  if [ -f "$_profile_file" ]; then
    printf '%s' "$_profile_file"
  elif [ -f "$_minimal_file" ]; then
    echo "[entrypoint] WARN: MCP profile ${_profile} not found; using minimal" >&2
    printf '%s' "$_minimal_file"
  else
    printf '%s' "$_legacy_file"
  fi
}

configure_claude_mcp() {
  [ -f "$HOME/.claude.json" ] || echo '{}' > "$HOME/.claude.json"
  [ -f "$MCP_SERVERS_FILE" ] || return 0
  mcp_rendered=$(mktemp)
  render_template_placeholders "$MCP_SERVERS_FILE" > "$mcp_rendered"
  mcp_merged=$(mktemp)
  if jq -s '
        .[0] as $cfg | .[1] as $servers
        | $cfg + { mcpServers: (($cfg.mcpServers // {}) * $servers) }
      ' "$HOME/.claude.json" "$mcp_rendered" > "$mcp_merged" 2>/dev/null; then
    mv "$mcp_merged" "$HOME/.claude.json"
  else
    echo "[entrypoint] WARN: failed to merge MCP servers from $MCP_SERVERS_FILE"
    rm -f "$mcp_merged"
  fi
  rm -f "$mcp_rendered"
}

configure_codex_mcp() {
  mkdir -p "$CODEX_HOME"
  [ -f "$CODEX_HOME/config.toml" ] || {
    cat > "$CODEX_HOME/config.toml" <<EOF
approval_policy = "never"
sandbox_mode = "danger-full-access"

[projects."$WORKSPACE_ROOT"]
trust_level = "trusted"
EOF
  }

  codex_tmp=$(mktemp)
  awk '
    /^\[mcp_servers\./ { skip = 1; next }
    /^\[features\]/    { skip = 1; next }
    /^\[apps\]/        { skip = 1; next }
    /^\[apps\./        { skip = 1; next }
    /^\[/              { skip = 0 }
    skip               { next }
    /^[[:space:]]*features\.apps[[:space:]]*=/ { next }
    /^[[:space:]]*apps\._default\./            { next }
    { print }
  ' "$CODEX_HOME/config.toml" > "$codex_tmp"
  {
    echo ""
    echo "[features]"
    echo "apps = false"
    echo ""
    echo "[apps._default]"
    echo "enabled = false"
    if [ -f "$CODEX_MCP_FILE" ]; then
      echo ""
      render_template_placeholders "$CODEX_MCP_FILE"
    fi
  } >> "$codex_tmp"
  awk 'NF{print; blank=0; next} {blank++} blank<2' "$codex_tmp" > "$CODEX_HOME/config.toml"
  rm -f "$codex_tmp"
}

clone_repo_into_workspace() {
  _repo_url="$1"
  _repo_branch="${2:-}"
  _repo_name="$(repo_dir_name "$_repo_url")"
  _repo_target="${WORKSPACE_ROOT}/${_repo_name}"

  if [ -d "${_repo_target}/.git" ]; then
    register_repo_trust "$_repo_target"
    return
  fi

  if [ -e "$_repo_target" ]; then
    echo "[entrypoint] WARN: ${_repo_target} exists but is not a git checkout; skipping clone of ${_repo_url}"
    return
  fi

  if [ -n "$_repo_branch" ]; then
    git clone --branch "$_repo_branch" "$_repo_url" "$_repo_target" ||
      echo "[entrypoint] WARN: clone of ${_repo_url} failed; continuing"
  else
    git clone "$_repo_url" "$_repo_target" ||
      echo "[entrypoint] WARN: clone of ${_repo_url} failed; continuing"
  fi
  [ -d "${_repo_target}/.git" ] && register_repo_trust "$_repo_target"
}

case "${AGENT_RUNNER_ENTRYPOINT_SELF_TEST:-}" in
  agent-kit-manifest)
    check_agent_kit_manifests
    exit 0
    ;;
  repo-allow)
    derive_repo_allow
    exit 0
    ;;
  repo-dir)
    repo_dir_name "${REPO_URL:-}"
    exit 0
    ;;
esac

check_agent_kit_manifests

export GIT_TERMINAL_PROMPT=0

AGENT_MCP_PROFILE="${AGENT_MCP_PROFILE:-minimal}"
case "$AGENT_MCP_PROFILE" in
  minimal|frontend|cluster|code-intel|full-diagnostic) ;;
  *)
    echo "[entrypoint] WARN: unknown AGENT_MCP_PROFILE=$AGENT_MCP_PROFILE; using minimal"
    AGENT_MCP_PROFILE="minimal"
    ;;
esac

AGENT_MCP_DIR="${AGENT_MCP_DIR:-}"
MCP_SERVERS_FILE="$(select_profile_file "claude-mcp-servers" "json" "${AGENT_MCP_SERVERS_FILE:-}" "$AGENT_MCP_PROFILE" "$AGENT_MCP_DIR")"
CODEX_MCP_FILE="$(select_profile_file "codex-mcp-servers" "toml" "${AGENT_CODEX_MCP_FILE:-}" "$AGENT_MCP_PROFILE" "$AGENT_MCP_DIR")"
if in_runner_container; then
  configure_global_git
  configure_claude_home
  configure_claude_mcp
  configure_codex_mcp
else
  echo "[entrypoint] WARN: not in the runner container; leaving the git, Claude and Codex config in HOME alone"
fi

if [ -n "${REPO_URL:-}" ]; then
  clone_repo_into_workspace "$REPO_URL" "${REPO_BRANCH:-}"
fi

REPO_ALLOW="$(derive_repo_allow)"
[ -z "$REPO_ALLOW" ] || export REPO_ALLOW

if [ -n "${REPO_URLS:-}" ]; then
  for entry in $(printf '%s' "$REPO_URLS" | tr ';\n' '  '); do
    repo_clone_url="${entry%%#*}"
    repo_branch=""
    case "$entry" in *"#"*) repo_branch="${entry#*#}" ;; esac
    clone_repo_into_workspace "$repo_clone_url" "$repo_branch"
  done
fi

if [ "$#" -gt 0 ]; then
  exec "$@"
fi
if [ -n "${AGENT_RUNNER_COMMAND:-}" ]; then
  exec sh -c "$AGENT_RUNNER_COMMAND"
fi
exec sh
