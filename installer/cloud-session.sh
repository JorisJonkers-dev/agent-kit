# shellcheck shell=bash
# setup-workstation.sh --cloud: a Claude Code cloud environment's setup script (docs/CLOUD.md).

AK_CLOUD_VARS=(HINDSIGHT_API_URL HINDSIGHT_API_TOKEN MEMORY_MCP_TOKEN OPENAI_API_KEY OPENROUTER_API_KEY OVERLEAF_SESSION)
AK_CLAUDE_CLI_PREFIX="${AK_CLAUDE_CLI_PREFIX:-/opt/agent-kit/claude-cli}"
AK_HERMES_MODEL="${AK_HERMES_MODEL:-deepseek/deepseek-v4-flash-0731}"

cloud_prepare() {
  local npm_prefix name visible=()
  log "cloud: running as $(id -un), HOME=${HOME}"

  npm_prefix="$(npm prefix -g 2>/dev/null || true)"
  export PATH="${AK_BIN_DIR}:${HOME}/.local/bin${npm_prefix:+:${npm_prefix}/bin}:${PATH}"
  # uv tools (hermes) default to ~/.local/bin, which the session's PATH may lack.
  export UV_TOOL_BIN_DIR="${AK_BIN_DIR}"

  for name in "${AK_CLOUD_VARS[@]}"; do
    if [ -n "${!name:-}" ]; then visible+=("${name}"); fi
  done
  log "cloud: environment variables visible to setup: ${visible[*]:-none}"

  # The host may not put its own claude on PATH before launch; registration needs one.
  if command -v claude >/dev/null 2>&1; then
    ok "cloud: claude $(claude --version 2>/dev/null | head -1) at $(command -v claude)"
  elif npm install -g --prefix "${AK_CLAUDE_CLI_PREFIX}" @anthropic-ai/claude-code >/dev/null 2>&1; then
    export PATH="${AK_CLAUDE_CLI_PREFIX}/bin:${PATH}"
    ok "cloud: configuration-only claude CLI in ${AK_CLAUDE_CLI_PREFIX}"
  else
    fail "cloud: no claude CLI and npm could not install one; plugins and MCP servers stay unregistered"
  fi
}

cloud_finish() {
  local hermes_config="${HERMES_HOME:-$HOME/.hermes}/config.yaml"

  if ! command -v codex >/dev/null 2>&1; then
    :
  elif codex login status >/dev/null 2>&1; then
    ok "cloud: codex is logged in"
  elif [ -n "${OPENAI_API_KEY:-}" ]; then
    # The login lands in ~/.codex/auth.json, so it survives into the snapshot.
    if printenv OPENAI_API_KEY | codex login --with-api-key >/dev/null 2>&1 \
       && codex login status >/dev/null 2>&1; then
      ok "cloud: codex logged in with OPENAI_API_KEY"
    else
      warn "cloud: codex login with OPENAI_API_KEY failed"
    fi
  else
    warn "cloud: codex is not logged in; in a session run: printenv OPENAI_API_KEY | codex login --with-api-key"
  fi

  if ! command -v hermes >/dev/null 2>&1; then
    :
  elif grep -q '^model:' "${hermes_config}" 2>/dev/null; then
    ok "cloud: hermes model already set in ${hermes_config}"
  else
    mkdir -p "$(dirname "${hermes_config}")"
    if [ -s "${hermes_config}" ] && [ -n "$(tail -c1 "${hermes_config}")" ]; then
      echo >> "${hermes_config}"
    fi
    printf '%s\n' 'model:' "  default: ${AK_HERMES_MODEL}" '  provider: openrouter' \
      '  base_url: https://openrouter.ai/api/v1' >> "${hermes_config}"
    if grep -q "^  default: ${AK_HERMES_MODEL}\$" "${hermes_config}"; then
      ok "cloud: hermes uses ${AK_HERMES_MODEL} through openrouter (reads OPENROUTER_API_KEY)"
    else
      fail "cloud: could not write the hermes model to ${hermes_config}"
    fi
  fi
}
