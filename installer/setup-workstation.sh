#!/usr/bin/env bash
# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.
#
# Estate workstation setup: Claude Code and Codex, their plugins, the
# language servers those plugins need, and the MCP fleet.
#
# Idempotent. Every step reports what it found rather than what it ran,
# because an installed plugin whose binary is absent looks identical to
# a working one from inside a session.
#
# Usage:
#   ./setup-workstation.sh                 install/upgrade everything
#   ./setup-workstation.sh --check         report only, change nothing
#   ./setup-workstation.sh --no-lsp        skip the language servers
#   ./setup-workstation.sh --no-mcp        skip MCP registration
#
# Secrets are read from the environment and never written here:
#   MEMORY_API_KEY  -> the memory MCP server
#   OVERLEAF_SESSION  -> the overleaf MCP server

set -uo pipefail

CHECK_ONLY=0
DO_LSP=1
DO_MCP=1
failures=0
warnings=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --check) CHECK_ONLY=1 ;;
    --no-lsp) DO_LSP=0 ;;
    --no-mcp) DO_MCP=0 ;;
    --help|-h) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
  shift
done

log()  { printf 'setup: %s\n' "$*"; }
ok()   { printf 'setup:   ok    %s\n' "$*"; }
warn() { printf 'setup:   warn  %s\n' "$*" >&2; warnings=$((warnings + 1)); }
fail() { printf 'setup:   FAIL  %s\n' "$*" >&2; failures=$((failures + 1)); }

run() {
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would run: $*"
    return 0
  fi
  "$@"
}

# Runs argv, and in --check mode prints only the first argument as a
# description. Used for the MCP registrations, whose arguments carry
# credentials: `run` would echo the secret in its `would run:` line.
run_redacted() {
  local what="$1"; shift
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would run: ${what}"
    return 0
  fi
  "$@"
}

# A shell -c wrapper for the pipe-into-sh installers the upstreams publish.
run_sh() {
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would run: $1"
    return 0
  fi
  bash -c "$1"
}

# This script lives in <kit>/installer, and the first-party skills it
# copies live in <kit>/skills.
KIT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"

# -----------------------------------------------------------------
# 1. Command-line tools.
# -----------------------------------------------------------------
log "command-line tools"

# Claude Code CLI.
if command -v claude >/dev/null 2>&1; then
  run_sh 'claude update'
  ok "claude $(claude --version 2>/dev/null | head -1)"
else
  run_sh 'curl -fsSL https://claude.ai/install.sh | bash'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "claude would be installed"
  elif command -v claude >/dev/null 2>&1; then
    ok "claude installed"
  else
    fail "claude still not on PATH after install; open a new shell and re-run"
  fi
fi

# OpenAI Codex CLI, the second engine council fans out to.
if command -v codex >/dev/null 2>&1; then
  run_sh 'npm install -g @openai/codex@latest'
  ok "codex $(codex --version 2>/dev/null | head -1)"
else
  run_sh 'npm install -g @openai/codex@latest'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "codex would be installed"
  elif command -v codex >/dev/null 2>&1; then
    ok "codex installed"
  else
    fail "codex still not on PATH after install; open a new shell and re-run"
  fi
fi

# Nous Research Hermes Agent. Installed on the workstation for
# `hermes doctor` / `hermes skills list` against a local config;
# the in-cluster gateway runs the container image, not this.
if command -v hermes >/dev/null 2>&1; then
  run_sh 'uv tool upgrade hermes-agent'
  ok "hermes $(hermes --version 2>/dev/null | head -1)"
else
  run_sh 'uv tool install --python 3.13 hermes-agent'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "hermes would be installed"
  elif command -v hermes >/dev/null 2>&1; then
    ok "hermes installed"
  else
    fail "hermes still not on PATH after install; open a new shell and re-run"
  fi
fi

# Overleaf CLI and MCP server. Points at the estate's self-hosted
# Overleaf, not overleaf.com.
if command -v olcli >/dev/null 2>&1; then
  run_sh 'npm install -g @aloth/olcli@latest'
  ok "olcli $(olcli --version 2>/dev/null | head -1)"
else
  run_sh 'npm install -g @aloth/olcli@latest'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "olcli would be installed"
  elif command -v olcli >/dev/null 2>&1; then
    ok "olcli installed"
  else
    fail "olcli still not on PATH after install; open a new shell and re-run"
  fi
fi

# Python tool/venv manager; prerequisite for hermes-agent and the
# kit's own tooling.
if command -v uv >/dev/null 2>&1; then
  run_sh 'uv self update'
  ok "uv $(uv --version 2>/dev/null | head -1)"
else
  run_sh 'curl -fsSL https://astral.sh/uv/install.sh | sh'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "uv would be installed"
  elif command -v uv >/dev/null 2>&1; then
    ok "uv installed"
  else
    fail "uv still not on PATH after install; open a new shell and re-run"
  fi
fi

# -----------------------------------------------------------------
# 2. Claude Code marketplaces and plugins.
# -----------------------------------------------------------------
if ! command -v claude >/dev/null 2>&1; then
  fail "claude is not on PATH; skipping plugins, language servers and MCP"
else
  log "plugin marketplaces"
  run claude plugin marketplace add anthropics/claude-plugins-official 2>/dev/null \
    || run claude plugin marketplace update claude-plugins-official >/dev/null 2>&1 \
    || warn "marketplace claude-plugins-official (anthropics/claude-plugins-official) could not be added or updated"
  run claude plugin marketplace add JuliusBrussee/caveman 2>/dev/null \
    || run claude plugin marketplace update caveman >/dev/null 2>&1 \
    || warn "marketplace caveman (JuliusBrussee/caveman) could not be added or updated"
  run claude plugin marketplace add jgraph/drawio-mcp 2>/dev/null \
    || run claude plugin marketplace update drawio >/dev/null 2>&1 \
    || warn "marketplace drawio (jgraph/drawio-mcp) could not be added or updated"
  run claude plugin marketplace add LukasNiessen/kubernetes-skill 2>/dev/null \
    || run claude plugin marketplace update kubernetes-skill >/dev/null 2>&1 \
    || warn "marketplace kubernetes-skill (LukasNiessen/kubernetes-skill) could not be added or updated"
  run claude plugin marketplace add codenamev/ai-software-architect 2>/dev/null \
    || run claude plugin marketplace update ai-software-architect >/dev/null 2>&1 \
    || warn "marketplace ai-software-architect (codenamev/ai-software-architect) could not be added or updated"

  log "plugins"
  # caveman: Compressed output mode plus the cavecrew subagents.
  if run claude plugin install caveman@caveman --yes --scope user; then
    run claude plugin update caveman >/dev/null 2>&1 || true
    run claude plugin enable caveman@caveman >/dev/null 2>&1 || true
    ok "caveman@caveman installed and enabled"
  else
    warn "caveman@caveman install failed"
  fi
  # drawio: Native .drawio diagram authoring and export.
  if run claude plugin install drawio@drawio --yes --scope user; then
    run claude plugin update drawio >/dev/null 2>&1 || true
    run claude plugin enable drawio@drawio >/dev/null 2>&1 || true
    ok "drawio@drawio installed and enabled"
  else
    warn "drawio@drawio install failed"
  fi
  # frontend-design: Visual design guidance for new UI work.
  if run claude plugin install frontend-design@claude-plugins-official --yes --scope user; then
    run claude plugin update frontend-design >/dev/null 2>&1 || true
    run claude plugin enable frontend-design@claude-plugins-official >/dev/null 2>&1 || true
    ok "frontend-design@claude-plugins-official installed and enabled"
  else
    warn "frontend-design@claude-plugins-official install failed"
  fi
  # mattpocock-skills: TDD, code review, domain modelling, grilling.
  if run claude plugin install mattpocock-skills@claude-plugins-official --yes --scope user; then
    run claude plugin update mattpocock-skills >/dev/null 2>&1 || true
    run claude plugin enable mattpocock-skills@claude-plugins-official >/dev/null 2>&1 || true
    ok "mattpocock-skills@claude-plugins-official installed and enabled"
  else
    warn "mattpocock-skills@claude-plugins-official install failed"
  fi
  # security-guidance: Security review of generated code. Ships its own PreToolUse and Stop hooks. Those are the PLUGIN's hooks, not estate hooks — the retired knowledge hooks are unrelated and the purge step matches on the
  if run claude plugin install security-guidance@claude-plugins-official --yes --scope user; then
    run claude plugin update security-guidance >/dev/null 2>&1 || true
    run claude plugin enable security-guidance@claude-plugins-official >/dev/null 2>&1 || true
    ok "security-guidance@claude-plugins-official installed and enabled"
  else
    warn "security-guidance@claude-plugins-official install failed"
  fi
  # kubernetes-skill: Manifest and cluster failure-mode guidance.
  if run claude plugin install kubernetes-skill@kubernetes-skill --yes --scope user; then
    run claude plugin update kubernetes-skill >/dev/null 2>&1 || true
    run claude plugin enable kubernetes-skill@kubernetes-skill >/dev/null 2>&1 || true
    ok "kubernetes-skill@kubernetes-skill installed and enabled"
  else
    warn "kubernetes-skill@kubernetes-skill install failed"
  fi
  # github: GitHub MCP server. Needs a token; it fails to connect with "Authorization header is badly formatted" when the credential is absent or malformed, which reads like a missing server rather than a missing
  if run claude plugin install github@claude-plugins-official --yes --scope user; then
    run claude plugin update github >/dev/null 2>&1 || true
    run claude plugin enable github@claude-plugins-official >/dev/null 2>&1 || true
    ok "github@claude-plugins-official installed and enabled"
  else
    warn "github@claude-plugins-official install failed"
  fi
  # ai-software-architect: Architecture review personas. Off by default — large surface, rarely the right tool.
  if run claude plugin install ai-software-architect@ai-software-architect --yes --scope user; then
    run claude plugin update ai-software-architect >/dev/null 2>&1 || true
    run claude plugin disable ai-software-architect@ai-software-architect >/dev/null 2>&1 || true
    ok "ai-software-architect@ai-software-architect installed, left disabled on purpose"
  else
    warn "ai-software-architect@ai-software-architect install failed"
  fi

  # ---------------------------------------------------------------
  # 3. Language servers: the plugin AND the binary it drives.
  #
  # An LSP plugin with no binary on PATH registers no tools and says
  # nothing about it, so the binary is what gets verified here.
  # ---------------------------------------------------------------
  if [ "${DO_LSP}" = 1 ]; then
    log "language servers"

    # typescript-lsp -> typescript-language-server (typescript, javascript)
    if run claude plugin install typescript-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update typescript-lsp >/dev/null 2>&1 || true
      run claude plugin enable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "typescript-lsp@claude-plugins-official install failed"
    fi
    if command -v typescript-language-server >/dev/null 2>&1; then
      ok "typescript-lsp: typescript-language-server on PATH"
    else
      run_sh 'npm install -g typescript-language-server typescript' \
        || warn "typescript-lsp: could not install typescript-language-server"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v typescript-language-server >/dev/null 2>&1; then
        warn "typescript-lsp: typescript-language-server still absent -- the plugin will register no tools"
      fi
    fi

    # pyright-lsp -> pyright-langserver (python)
    if run claude plugin install pyright-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update pyright-lsp >/dev/null 2>&1 || true
      run claude plugin enable pyright-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "pyright-lsp@claude-plugins-official install failed"
    fi
    if command -v pyright-langserver >/dev/null 2>&1; then
      ok "pyright-lsp: pyright-langserver on PATH"
    else
      run_sh 'npm install -g pyright' \
        || warn "pyright-lsp: could not install pyright-langserver"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v pyright-langserver >/dev/null 2>&1; then
        warn "pyright-lsp: pyright-langserver still absent -- the plugin will register no tools"
      fi
    fi

    # kotlin-lsp -> kotlin-lsp (kotlin)
    if run claude plugin install kotlin-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update kotlin-lsp >/dev/null 2>&1 || true
      run claude plugin enable kotlin-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "kotlin-lsp@claude-plugins-official install failed"
    fi
    if command -v kotlin-lsp >/dev/null 2>&1; then
      ok "kotlin-lsp: kotlin-lsp on PATH"
    else
      run_sh 'brew install kotlin-lsp' \
        || warn "kotlin-lsp: could not install kotlin-lsp"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v kotlin-lsp >/dev/null 2>&1; then
        warn "kotlin-lsp: kotlin-lsp still absent -- the plugin will register no tools"
      fi
    fi

    # jdtls-lsp -> jdtls (java)
    if run claude plugin install jdtls-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update jdtls-lsp >/dev/null 2>&1 || true
      run claude plugin enable jdtls-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "jdtls-lsp@claude-plugins-official install failed"
    fi
    if command -v jdtls >/dev/null 2>&1; then
      ok "jdtls-lsp: jdtls on PATH"
    else
      run_sh 'brew install jdtls' \
        || warn "jdtls-lsp: could not install jdtls"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v jdtls >/dev/null 2>&1; then
        warn "jdtls-lsp: jdtls still absent -- the plugin will register no tools"
      fi
    fi

    # ruby-lsp -> ruby-lsp (ruby)
    if run claude plugin install ruby-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update ruby-lsp >/dev/null 2>&1 || true
      run claude plugin enable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "ruby-lsp@claude-plugins-official install failed"
    fi
    if command -v ruby-lsp >/dev/null 2>&1; then
      ok "ruby-lsp: ruby-lsp on PATH"
    else
      run_sh 'gem install ruby-lsp' \
        || warn "ruby-lsp: could not install ruby-lsp"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v ruby-lsp >/dev/null 2>&1; then
        warn "ruby-lsp: ruby-lsp still absent -- the plugin will register no tools"
      fi
    fi

    # gopls-lsp -> gopls (go)
    if run claude plugin install gopls-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update gopls-lsp >/dev/null 2>&1 || true
      run claude plugin enable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "gopls-lsp@claude-plugins-official install failed"
    fi
    if command -v gopls >/dev/null 2>&1; then
      ok "gopls-lsp: gopls on PATH"
    else
      run_sh 'go install golang.org/x/tools/gopls@latest' \
        || warn "gopls-lsp: could not install gopls"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v gopls >/dev/null 2>&1; then
        warn "gopls-lsp: gopls still absent -- the plugin will register no tools"
      fi
    fi

    # rust-analyzer-lsp -> rust-analyzer (rust)
    if run claude plugin install rust-analyzer-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update rust-analyzer-lsp >/dev/null 2>&1 || true
      run claude plugin enable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "rust-analyzer-lsp@claude-plugins-official install failed"
    fi
    if command -v rust-analyzer >/dev/null 2>&1; then
      ok "rust-analyzer-lsp: rust-analyzer on PATH"
    else
      run_sh 'rustup component add rust-analyzer' \
        || warn "rust-analyzer-lsp: could not install rust-analyzer"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v rust-analyzer >/dev/null 2>&1; then
        warn "rust-analyzer-lsp: rust-analyzer still absent -- the plugin will register no tools"
      fi
    fi

    # lua-lsp -> lua-language-server (lua)
    if run claude plugin install lua-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update lua-lsp >/dev/null 2>&1 || true
      run claude plugin enable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "lua-lsp@claude-plugins-official install failed"
    fi
    if command -v lua-language-server >/dev/null 2>&1; then
      ok "lua-lsp: lua-language-server on PATH"
    else
      run_sh 'brew install lua-language-server' \
        || warn "lua-lsp: could not install lua-language-server"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v lua-language-server >/dev/null 2>&1; then
        warn "lua-lsp: lua-language-server still absent -- the plugin will register no tools"
      fi
    fi

    # clangd-lsp -> clangd (c, cpp)
    if run claude plugin install clangd-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update clangd-lsp >/dev/null 2>&1 || true
      run claude plugin enable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "clangd-lsp@claude-plugins-official install failed"
    fi
    if command -v clangd >/dev/null 2>&1; then
      ok "clangd-lsp: clangd on PATH"
    else
      run_sh 'brew install llvm' \
        || warn "clangd-lsp: could not install clangd"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v clangd >/dev/null 2>&1; then
        warn "clangd-lsp: clangd still absent -- the plugin will register no tools"
      fi
    fi

    # php-lsp -> intelephense (php)
    if run claude plugin install php-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update php-lsp >/dev/null 2>&1 || true
      run claude plugin enable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "php-lsp@claude-plugins-official install failed"
    fi
    if command -v intelephense >/dev/null 2>&1; then
      ok "php-lsp: intelephense on PATH"
    else
      run_sh 'npm install -g intelephense' \
        || warn "php-lsp: could not install intelephense"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v intelephense >/dev/null 2>&1; then
        warn "php-lsp: intelephense still absent -- the plugin will register no tools"
      fi
    fi

    # csharp-lsp -> csharp-ls (csharp)
    if run claude plugin install csharp-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update csharp-lsp >/dev/null 2>&1 || true
      run claude plugin enable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "csharp-lsp@claude-plugins-official install failed"
    fi
    if command -v csharp-ls >/dev/null 2>&1; then
      ok "csharp-lsp: csharp-ls on PATH"
    else
      run_sh 'dotnet tool install --global csharp-ls' \
        || warn "csharp-lsp: could not install csharp-ls"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v csharp-ls >/dev/null 2>&1; then
        warn "csharp-lsp: csharp-ls still absent -- the plugin will register no tools"
      fi
    fi

    # swift-lsp -> sourcekit-lsp (swift)
    if run claude plugin install swift-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update swift-lsp >/dev/null 2>&1 || true
      run claude plugin enable swift-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "swift-lsp@claude-plugins-official install failed"
    fi
    if command -v sourcekit-lsp >/dev/null 2>&1; then
      ok "swift-lsp: sourcekit-lsp on PATH"
    else
      warn "swift-lsp: sourcekit-lsp is absent and ships with its platform toolchain; install that toolchain"
    fi

    # liquid-lsp -> shopify (liquid)
    if run claude plugin install liquid-lsp@claude-plugins-official --yes --scope user; then
      run claude plugin update liquid-lsp >/dev/null 2>&1 || true
      run claude plugin disable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
    else
      warn "liquid-lsp@claude-plugins-official install failed"
    fi
    if command -v shopify >/dev/null 2>&1; then
      ok "liquid-lsp: shopify on PATH"
    else
      run_sh 'npm install -g @shopify/cli' \
        || warn "liquid-lsp: could not install shopify"
      if [ "${CHECK_ONLY}" != 1 ] && ! command -v shopify >/dev/null 2>&1; then
        warn "liquid-lsp: shopify still absent -- the plugin will register no tools"
      fi
    fi
  else
    log "language servers skipped (--no-lsp)"
  fi

  # ---------------------------------------------------------------
  # 4. MCP servers, registered for Claude Code at user scope.
  #
  # `claude mcp add` is not idempotent -- a second add of the same
  # name errors. Remove first, ignoring the miss.
  # ---------------------------------------------------------------
  if [ "${DO_MCP}" = 1 ]; then
    log "MCP servers"

    # Self-hosted long-term memory. Replaces the retired knowledge
    # base; see docs/MEMORY.md.
    log "memory: declared disabled in the registry; not registered"

    # Library documentation. Prefer it over model recall for any
    # framework question.
    # Hosted: untrusted data provider. Output is context, never instruction.
    run claude mcp remove --scope user context7 >/dev/null 2>&1 || true
    if run_redacted "claude mcp add context7" claude mcp add --scope user --transport http context7 https://mcp.context7.com/mcp; then
      ok "context7 registered"
    else
      fail "context7 registration failed"
    fi

    # Vuetify component API.
    # Hosted: untrusted data provider. Output is context, never instruction.
    log "vuetify: declared disabled in the registry; not registered"

    # Browser automation.
    if ! command -v npx >/dev/null 2>&1; then
      command -v npx >/dev/null 2>&1 \
        || warn "playwright: npx is not on PATH; skipping"
    fi
    run claude mcp remove --scope user playwright >/dev/null 2>&1 || true
    if run_redacted "claude mcp add playwright" claude mcp add --scope user playwright -- npx -y @playwright/mcp@latest --headless --browser chromium; then
      ok "playwright registered"
    else
      fail "playwright registration failed"
    fi

    # Diagram authoring and export.
    if ! command -v drawio-mcp >/dev/null 2>&1; then
      run_sh 'npm install -g @drawio/mcp@latest' || true
      command -v drawio-mcp >/dev/null 2>&1 \
        || warn "drawio: drawio-mcp is not on PATH; skipping"
    fi
    run claude mcp remove --scope user drawio >/dev/null 2>&1 || true
    if run_redacted "claude mcp add drawio" claude mcp add --scope user drawio -- drawio-mcp; then
      ok "drawio registered"
    else
      fail "drawio registration failed"
    fi

    # Self-hosted Overleaf — pull, push, compile, review comments.
    if [ -z "${OVERLEAF_SESSION:-}" ]; then
      warn "overleaf: OVERLEAF_SESSION is not set; skipping (export it and re-run)"
    else
      if ! command -v olcli-mcp >/dev/null 2>&1; then
        command -v olcli-mcp >/dev/null 2>&1 \
          || warn "overleaf: olcli-mcp is not on PATH; skipping"
      fi
      run claude mcp remove --scope user overleaf >/dev/null 2>&1 || true
      if run_redacted "claude mcp add overleaf" claude mcp add --scope user --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf_session2" --env OVERLEAF_SESSION="${OVERLEAF_SESSION}" overleaf -- olcli-mcp; then
        ok "overleaf registered"
      else
        fail "overleaf registration failed"
      fi
    fi

    # Verify the VALUE, not the exit codes above: ask Claude what it
    # actually has registered and name anything expected but absent.
    if [ "${CHECK_ONLY}" != 1 ]; then
      registered=$(claude mcp list 2>/dev/null || true)
      for want in \
        context7 \
        playwright \
        drawio \
        overleaf \
        ; do
        case "${registered}" in
          *"${want}"*) ;;
          *) warn "MCP server ${want} is not in \`claude mcp list\` output" ;;
        esac
      done
    fi
  else
    log "MCP registration skipped (--no-mcp)"
  fi
fi

# -----------------------------------------------------------------
# 5. First-party skills that ship in this repository.
#
# Copied, not fetched. Multi-file skills, so the whole tree moves and
# the destination is replaced rather than merged -- a stale script left
# behind from an older version is worse than a missing one.
# -----------------------------------------------------------------
log "first-party skills"

# Compose a pull request in the repository's own house style:
# reads the PR template, mines recent merged PRs, and gates on an
# explicit confirmation before anything is pushed or created.
src="${KIT_ROOT}/skills/pr-composer"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "pr-composer: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/pr-composer"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "pr-composer -> ${dest}"
    else
      fail "pr-composer: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/pr-composer"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "pr-composer -> ${dest}"
    else
      fail "pr-composer: ${dest} is incomplete after copy"
    fi
  fi
fi

# -----------------------------------------------------------------
# 6. Retired hooks.
#
# The estate ships no agent hooks. install-agents.sh owns the purge;
# this only reports a machine that still has them so the operator
# knows to run it.
# -----------------------------------------------------------------
settings="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json"
if [ -f "${settings}" ] && grep -qE "pre-tool-use-edit-recall|pre-tool-use-git-commit-capture|stop-session-digest|kb-stop-digest|user-prompt-submit-recall" "${settings}"; then
  warn "retired knowledge hooks are still wired in ${settings}; run install-agents.sh to purge them"
else
  ok "no retired knowledge hooks in ${settings}"
fi

# -----------------------------------------------------------------
log "summary: ${failures} failure(s), ${warnings} warning(s)"
[ "${failures}" = 0 ]
