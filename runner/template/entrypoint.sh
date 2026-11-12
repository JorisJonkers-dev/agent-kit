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

# Overlay the newest published kit onto the baked one. The platform names the
# runtime-home artifact in AGENT_KIT_RUNTIME_REF (tag or digest reference); oras
# pulls it and nothing is installed until the tarball matches the .sha256 the
# release published beside it, the optional AGENT_KIT_RUNTIME_DIGEST the
# platform pinned, and the SHA256SUMS inside it. The .sha256 travels in the
# same artifact, so it catches corruption; authenticity comes from pinning
# AGENT_KIT_RUNTIME_DIGEST (or naming the artifact by @sha256: digest, which
# oras verifies itself). Any failure leaves the baked
# kit untouched and says so; the session reports the version it ended up with.
KIT_FETCH_DIR="${AGENT_KIT_FETCH_DIR:-${HOME}/.agent-kit-fetched}"

kit_fallback() {
  _baked="$(awk -F= '/^version=/{print $2; exit}' "${CLAUDE_CONFIG_DIR}/${AGENT_KIT_VERSION_MARKER}" 2>/dev/null || true)"
  _baked="${_baked:-unknown}"
  echo "[entrypoint] WARN: kit fetch failed: $1; falling back to the baked kit (version ${_baked})"
  mkdir -p "$KIT_FETCH_DIR" 2>/dev/null &&
    printf 'source=baked\nversion=%s\nreason=%s\n' "$_baked" "$1" > "$KIT_FETCH_DIR/status" 2>/dev/null || true
  rm -rf "${_kit_stage:-}"
  return 0
}

fetch_newest_kit() {
  _ref="${AGENT_KIT_RUNTIME_REF:-}"
  if [ -z "$_ref" ]; then
    echo "[entrypoint] kit fetch not configured (AGENT_KIT_RUNTIME_REF unset); using the baked kit"
    return 0
  fi
  if [ "${AGENT_KIT_FETCH:-1}" = 0 ]; then
    echo "[entrypoint] kit fetch disabled (AGENT_KIT_FETCH=0); using the baked kit"
    return 0
  fi
  _kit_stage=""
  command -v oras >/dev/null 2>&1 || { kit_fallback "oras is not on PATH"; return 0; }
  command -v sha256sum >/dev/null 2>&1 || { kit_fallback "sha256sum is not on PATH"; return 0; }
  _kit_stage="$(mktemp -d)" || { kit_fallback "cannot create a staging directory"; return 0; }
  mkdir -p "$_kit_stage/pull" "$_kit_stage/kit"
  oras pull "$_ref" -o "$_kit_stage/pull" >"$_kit_stage/oras.log" 2>&1 ||
    { kit_fallback "oras pull of ${_ref} failed: $(tail -n 1 "$_kit_stage/oras.log")"; return 0; }

  _tarballs="$(find "$_kit_stage/pull" -maxdepth 1 -type f -name 'agent-kit-runtime-home-v*.tar.gz')"
  case "$_tarballs" in
    ""|*"
"*) kit_fallback "${_ref} did not hold exactly one runtime-home tarball"; return 0 ;;
  esac
  _tarball="$_tarballs"
  _version="${_tarball##*/agent-kit-runtime-home-}"
  _version="${_version%.tar.gz}"
  _published="$(awk '{print $1; exit}' "${_tarball}.sha256" 2>/dev/null || true)"
  _actual="$(sha256sum "$_tarball" | awk '{print $1}')"
  [ -n "$_published" ] || { kit_fallback "${_ref} published no sha256 for ${_version}"; return 0; }
  [ "$_published" = "$_actual" ] ||
    { kit_fallback "sha256 of ${_version} is ${_actual}, the release published ${_published}"; return 0; }
  if [ -n "${AGENT_KIT_RUNTIME_DIGEST:-}" ] && [ "${AGENT_KIT_RUNTIME_DIGEST#sha256:}" != "$_actual" ]; then
    kit_fallback "sha256 of ${_version} is ${_actual}, not the pinned ${AGENT_KIT_RUNTIME_DIGEST}"
    return 0
  fi

  _entries="$(tar -tzf "$_tarball" 2>/dev/null)" || { kit_fallback "${_version} is not a readable tarball"; return 0; }
  case "
$_entries
" in
    *"
/"*|*"../"*|*"/.."*) kit_fallback "${_version} holds an absolute or parent-relative path"; return 0 ;;
  esac
  # Only files and directories: a link or device member could point the
  # extraction outside the staging directory.
  if tar -tvzf "$_tarball" 2>/dev/null | grep -Eq '^[^-d]| -> | link to '; then
    kit_fallback "${_version} holds a link or special file"
    return 0
  fi
  tar -xzf "$_tarball" -C "$_kit_stage/kit" || { kit_fallback "cannot extract ${_version}"; return 0; }
  (cd "$_kit_stage/kit" && sha256sum -c SHA256SUMS >/dev/null 2>&1) ||
    { kit_fallback "SHA256SUMS inside ${_version} does not match its files"; return 0; }
  [ -d "$_kit_stage/kit/home" ] || { kit_fallback "${_version} has no home directory"; return 0; }

  mkdir -p "$HOME" "$KIT_FETCH_DIR" || { kit_fallback "cannot write ${HOME} or ${KIT_FETCH_DIR}"; return 0; }
  # Credentials stay as the runner mounted them.
  (cd "$_kit_stage/kit/home" && tar -cf "$_kit_stage/overlay.tar" \
    --exclude=./.claude/.credentials.json --exclude=./.codex/auth.json --exclude=./.codex/config.toml .) ||
    { kit_fallback "cannot stage the home of ${_version}"; return 0; }
  (cd "$HOME" && tar -xf "$_kit_stage/overlay.tar") ||
    { kit_fallback "cannot install the home of ${_version}; ${HOME} may be partly overlaid"; return 0; }
  if [ -d "$_kit_stage/kit/mcp" ]; then
    rm -rf "$KIT_FETCH_DIR/mcp"
    cp -R "$_kit_stage/kit/mcp" "$KIT_FETCH_DIR/mcp" || { kit_fallback "cannot install the MCP profiles of ${_version}"; return 0; }
    AGENT_MCP_DIR="$KIT_FETCH_DIR/mcp"
    export AGENT_MCP_DIR
  fi
  mkdir -p "$CLAUDE_CONFIG_DIR" "$CODEX_HOME"
  printf 'version=%s\n' "$_version" > "${CLAUDE_CONFIG_DIR}/${AGENT_KIT_VERSION_MARKER}"
  printf 'version=%s\n' "$_version" > "${CODEX_HOME}/${AGENT_KIT_VERSION_MARKER}"
  printf 'source=fetched\nversion=%s\nsha256=%s\nref=%s\n' "$_version" "$_actual" "$_ref" > "$KIT_FETCH_DIR/status"
  AGENT_KIT_FETCHED_VERSION="$_version"
  export AGENT_KIT_FETCHED_VERSION
  rm -rf "$_kit_stage"
  echo "[entrypoint] kit fetched: agent-kit ${_version} (sha256 ${_actual} verified) from ${_ref}"
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

if in_runner_container; then
  fetch_newest_kit
fi
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
