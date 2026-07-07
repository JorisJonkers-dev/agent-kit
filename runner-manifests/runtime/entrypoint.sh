#!/bin/sh
set -eu

CODEX_HOME="${CODEX_HOME:-${HOME}/.codex}"
CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-${HOME}/.claude}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-$PWD}"
AGENT_KIT_SDD_SOURCE="${AGENT_KIT_SDD_SOURCE:-}"
AGENT_KIT_SDD_MARKER_FILE="${AGENT_KIT_SDD_MARKER_FILE:-.agent-kit-sdd-seed.sha256}"
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

register_repo_trust() {
  _dir="$1"
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

speckit_marker_hash() {
  _speckit_path="$1"
  _speckit_marker="$2"
  if [ -f "$_speckit_marker" ]; then
    awk -v p="$_speckit_path" '$2 == p { print $1; exit }' "$_speckit_marker"
  fi
}

speckit_seed_file() {
  _speckit_src="$1"
  _speckit_dest="$2"
  _speckit_rel="$3"
  _speckit_src_hash="$(sha256sum "$_speckit_src" | awk '{ print $1 }')"
  _speckit_old_hash="$(speckit_marker_hash "$_speckit_rel" "$_speckit_marker")"
  _speckit_record_hash=""

  mkdir -p "$(dirname "$_speckit_dest")"
  if [ ! -f "$_speckit_dest" ]; then
    cp -p "$_speckit_src" "$_speckit_dest"
    _speckit_record_hash="$_speckit_src_hash"
  elif [ -n "$_speckit_old_hash" ]; then
    _speckit_dest_hash="$(sha256sum "$_speckit_dest" | awk '{ print $1 }')"
    if [ "$_speckit_dest_hash" = "$_speckit_old_hash" ]; then
      [ "$_speckit_dest_hash" = "$_speckit_src_hash" ] || cp -p "$_speckit_src" "$_speckit_dest"
      _speckit_record_hash="$_speckit_src_hash"
    else
      if [ "$_speckit_old_hash" != "$_speckit_src_hash" ]; then
        echo "[entrypoint] WARN: ${_speckit_dest} has local changes and a stale SDD seed; leaving it unchanged"
      fi
      _speckit_record_hash="$_speckit_old_hash"
    fi
  else
    _speckit_dest_hash="$(sha256sum "$_speckit_dest" | awk '{ print $1 }')"
    [ "$_speckit_dest_hash" = "$_speckit_src_hash" ] && _speckit_record_hash="$_speckit_src_hash"
  fi

  if [ -n "$_speckit_record_hash" ]; then
    printf '%s %s\n' "$_speckit_record_hash" "$_speckit_rel" >> "$_speckit_new_marker"
  fi
}

speckit_seed() {
  _speckit_repo="$1"
  _speckit_dest_root="${_speckit_repo}/.specify"
  _speckit_marker="${_speckit_dest_root}/${AGENT_KIT_SDD_MARKER_FILE}"

  if [ -z "$AGENT_KIT_SDD_SOURCE" ] || [ ! -d "$AGENT_KIT_SDD_SOURCE" ]; then
    echo "[entrypoint] WARN: SDD source missing; skipping ${_speckit_repo}"
    return
  fi

  _speckit_new_marker="$(mktemp)"
  for _speckit_dir in templates scripts; do
    if [ -d "${AGENT_KIT_SDD_SOURCE}/${_speckit_dir}" ]; then
      find "${AGENT_KIT_SDD_SOURCE}/${_speckit_dir}" -type f | sort | while IFS= read -r _speckit_src; do
        _speckit_rel="${_speckit_src#${AGENT_KIT_SDD_SOURCE}/}"
        speckit_seed_file "$_speckit_src" "${_speckit_dest_root}/${_speckit_rel}" "$_speckit_rel"
      done
    fi
  done

  if [ -f "${AGENT_KIT_SDD_SOURCE}/templates/constitution-template.md" ]; then
    speckit_seed_file \
      "${AGENT_KIT_SDD_SOURCE}/templates/constitution-template.md" \
      "${_speckit_dest_root}/memory/constitution.md" \
      "memory/constitution.md"
  fi

  if [ -s "$_speckit_new_marker" ]; then
    mkdir -p "$_speckit_dest_root"
    mv "$_speckit_new_marker" "$_speckit_marker"
  else
    rm -f "$_speckit_new_marker"
  fi
}

speckit_seed_workspace() {
  for _speckit_git_dir in "${WORKSPACE_ROOT}"/*/.git; do
    [ -d "$_speckit_git_dir" ] || continue
    speckit_seed "${_speckit_git_dir%/.git}"
  done
}

render_template_placeholders() {
  sed -e "s|@KNOWLEDGE_MCP_URL@|${KB_URL:-}|g" \
      -e "s|@KNOWLEDGE_MCP_BEARER_TOKEN@|${KB_BEARER_TOKEN:-}|g" \
      -e "s|@CLUSTER_MCP_URL@|${AGENT_CLUSTER_MCP_URL:-}|g" \
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
  [ -f "$MCP_SERVERS_FILE" ] || return
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
  speckit-seed)
    speckit_seed_workspace
    exit 0
    ;;
esac

check_agent_kit_manifests

if command -v git >/dev/null 2>&1; then
  git config --global user.name "${GIT_AUTHOR_NAME:-JorisJonkers Agent}"
  git config --global user.email "${GIT_AUTHOR_EMAIL:-agents@jorisjonkers.dev}"
  git config --global init.defaultBranch "${GIT_DEFAULT_BRANCH:-main}"
  git config --global credential.helper agent-gh-app
  git config --global credential.useHttpPath true
  export GIT_TERMINAL_PROMPT=0
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
fi

if [ ! -f "$HOME/.claude.json" ]; then
  echo '{}' > "$HOME/.claude.json"
fi
if command -v jq >/dev/null 2>&1; then
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
fi

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
configure_claude_mcp
configure_codex_mcp

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

speckit_seed_workspace

if [ "$#" -gt 0 ]; then
  exec "$@"
fi
if [ -n "${AGENT_RUNNER_COMMAND:-}" ]; then
  exec sh -c "$AGENT_RUNNER_COMMAND"
fi
exec sh
