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

# Track what needs attention after the run
skipped_mcp_servers=()     # MCP servers skipped due to missing credentials
missing_lsp_binaries=()    # Language servers with missing binaries
disabled_plugins=()        # Plugins disabled (missing binary or on purpose)
plugin_drift=()            # Plugins whose commit drifted
new_binaries=()            # Binaries installed during this run

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
    new_binaries+=("claude")
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
    new_binaries+=("codex")
  else
    fail "codex still not on PATH after install; open a new shell and re-run"
  fi
fi

# Nous Research Hermes Agent. Installed on the workstation for
# `hermes doctor` / `hermes skills list` against a local config;
# the in-cluster gateway runs the container image, not this.
if command -v hermes >/dev/null 2>&1; then
  run_sh 'uv tool upgrade '\''hermes-agent[mcp]'\'''
  ok "hermes $(hermes --version 2>/dev/null | head -1)"
else
  run_sh 'uv tool install --python 3.13 '\''hermes-agent[mcp]'\'''
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "hermes would be installed"
  elif command -v hermes >/dev/null 2>&1; then
    ok "hermes installed"
    new_binaries+=("hermes")
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
    new_binaries+=("olcli")
    new_binaries+=("olcli-mcp")
    new_binaries+=("git-remote-overleaf")
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
    new_binaries+=("uv")
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
    run claude plugin enable caveman@caveman >/dev/null 2>&1 || true
    ok "caveman@caveman installed and enabled"
  else
    warn "caveman@caveman install failed"
  fi
  # drawio: Native .drawio diagram authoring and export.
  if run claude plugin install drawio@drawio --yes --scope user; then
    run claude plugin enable drawio@drawio >/dev/null 2>&1 || true
    ok "drawio@drawio installed and enabled"
  else
    warn "drawio@drawio install failed"
  fi
  # frontend-design: Visual design guidance for new UI work.
  if run claude plugin install frontend-design@claude-plugins-official --yes --scope user; then
    run claude plugin enable frontend-design@claude-plugins-official >/dev/null 2>&1 || true
    ok "frontend-design@claude-plugins-official installed and enabled"
  else
    warn "frontend-design@claude-plugins-official install failed"
  fi
  # mattpocock-skills: TDD, code review, domain modelling, grilling.
  if run claude plugin install mattpocock-skills@claude-plugins-official --yes --scope user; then
    run claude plugin enable mattpocock-skills@claude-plugins-official >/dev/null 2>&1 || true
    ok "mattpocock-skills@claude-plugins-official installed and enabled"
  else
    warn "mattpocock-skills@claude-plugins-official install failed"
  fi
  # security-guidance: Security review of generated code. Ships its own PreToolUse and Stop hooks. Those are the PLUGIN's hooks, not estate hooks — the retired knowledge hooks are unrelated and the purge step matches on the
  if run claude plugin install security-guidance@claude-plugins-official --yes --scope user; then
    run claude plugin enable security-guidance@claude-plugins-official >/dev/null 2>&1 || true
    ok "security-guidance@claude-plugins-official installed and enabled"
  else
    warn "security-guidance@claude-plugins-official install failed"
  fi
  # kubernetes-skill: Manifest and cluster failure-mode guidance.
  if run claude plugin install kubernetes-skill@kubernetes-skill --yes --scope user; then
    run claude plugin enable kubernetes-skill@kubernetes-skill >/dev/null 2>&1 || true
    ok "kubernetes-skill@kubernetes-skill installed and enabled"
  else
    warn "kubernetes-skill@kubernetes-skill install failed"
  fi
  # github: GitHub MCP server. Needs a token; it fails to connect with "Authorization header is badly formatted" when the credential is absent or malformed, which reads like a missing server rather than a missing
  if run claude plugin install github@claude-plugins-official --yes --scope user; then
    run claude plugin enable github@claude-plugins-official >/dev/null 2>&1 || true
    ok "github@claude-plugins-official installed and enabled"
  else
    warn "github@claude-plugins-official install failed"
  fi
  # ai-software-architect: Architecture review personas. Off by default — large surface, rarely the right tool.
  if run claude plugin install ai-software-architect@ai-software-architect --yes --scope user; then
    run claude plugin disable ai-software-architect@ai-software-architect >/dev/null 2>&1 || true
    ok "ai-software-architect@ai-software-architect installed, left disabled on purpose"
    disabled_plugins+=("ai-software-architect: disabled on purpose in registry")
  else
    warn "ai-software-architect@ai-software-architect install failed"
  fi

  # ---------------------------------------------------------------
  # 3. Plugin drift detection.
  #
  # The CLI does not support pinning plugins to a commit, so pins in
  # the registry are advisory. Compare installed commits against
  # registry expectations and report drift.
  # ---------------------------------------------------------------
  manifest="${CLAUDE_HOME}/plugins/installed_plugins.json"
  if [ -f "${manifest}" ]; then
    log "checking plugin commit drift"

    # The reader is a helper taking the manifest path and the plugin
    # ref as ARGV, with SINGLE-quoted python source. An earlier
    # version inlined double-quoted python inside a double-quoted
    # shell string, so the inner quotes closed the shell string and
    # python got mangled source -- and `2>/dev/null || true` hid the
    # SyntaxError, so every plugin silently reported no drift.
    installed_plugin_commit() {
      python3 -c '
import json, sys
try:
    manifest = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
installs = manifest.get("plugins", {}).get(sys.argv[2], [])
for install in installs:
    if install.get("scope") == "user":
        print(install.get("gitCommitSha", ""))
        break
' \
        "$1" "$2" 2>/dev/null
    }

    installed_commit="$(installed_plugin_commit "${manifest}" "caveman@caveman")"
    if [ -z "${installed_commit}" ]; then
      log "caveman: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "b82c0ad42c2bedc1f2cd78e414dadfaffbaaeec3" ]; then
      warn "caveman: installed ${installed_commit} differs from the registry's b82c0ad42c2b"
      plugin_drift+=("caveman: ${installed_commit} vs b82c0ad42c2b")
    else
      ok "caveman: at the expected commit"
    fi
    installed_commit="$(installed_plugin_commit "${manifest}" "drawio@drawio")"
    if [ -z "${installed_commit}" ]; then
      log "drawio: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "14b318b19cc37b159f841227b9d11fbd18ce18ea" ]; then
      warn "drawio: installed ${installed_commit} differs from the registry's 14b318b19cc3"
      plugin_drift+=("drawio: ${installed_commit} vs 14b318b19cc3")
    else
      ok "drawio: at the expected commit"
    fi
    installed_commit="$(installed_plugin_commit "${manifest}" "frontend-design@claude-plugins-official")"
    if [ -z "${installed_commit}" ]; then
      log "frontend-design: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "0d82eac145a50e6867d908419dccc5087b8595b0" ]; then
      warn "frontend-design: installed ${installed_commit} differs from the registry's 0d82eac145a5"
      plugin_drift+=("frontend-design: ${installed_commit} vs 0d82eac145a5")
    else
      ok "frontend-design: at the expected commit"
    fi
    installed_commit="$(installed_plugin_commit "${manifest}" "mattpocock-skills@claude-plugins-official")"
    if [ -z "${installed_commit}" ]; then
      log "mattpocock-skills: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "5b15a47f2d7150f545fbcacbfe381787fc0230dc" ]; then
      warn "mattpocock-skills: installed ${installed_commit} differs from the registry's 5b15a47f2d71"
      plugin_drift+=("mattpocock-skills: ${installed_commit} vs 5b15a47f2d71")
    else
      ok "mattpocock-skills: at the expected commit"
    fi
    installed_commit="$(installed_plugin_commit "${manifest}" "kubernetes-skill@kubernetes-skill")"
    if [ -z "${installed_commit}" ]; then
      log "kubernetes-skill: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "f85547fb3a1ec909b2cbe4dc68f831590ac385ea" ]; then
      warn "kubernetes-skill: installed ${installed_commit} differs from the registry's f85547fb3a1e"
      plugin_drift+=("kubernetes-skill: ${installed_commit} vs f85547fb3a1e")
    else
      ok "kubernetes-skill: at the expected commit"
    fi
    installed_commit="$(installed_plugin_commit "${manifest}" "ai-software-architect@ai-software-architect")"
    if [ -z "${installed_commit}" ]; then
      log "ai-software-architect: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "6e636c8bb2f63481d70f0b7af12912fd414d5722" ]; then
      warn "ai-software-architect: installed ${installed_commit} differs from the registry's 6e636c8bb2f6"
      plugin_drift+=("ai-software-architect: ${installed_commit} vs 6e636c8bb2f6")
    else
      ok "ai-software-architect: at the expected commit"
    fi
  fi

  # ---------------------------------------------------------------
  # 3. Language servers: the plugin AND the binary it drives.
  #
  # An LSP plugin with no binary on PATH registers no tools and says
  # nothing about it. Install the plugin, but leave it disabled until
  # the binary is on PATH. This enables it on a second run once the
  # binary is installed.
  # ---------------------------------------------------------------
  if [ "${DO_LSP}" = 1 ]; then
    log "language servers"

    # typescript-lsp -> typescript-language-server (typescript, javascript)
    if run claude plugin install typescript-lsp@claude-plugins-official --yes --scope user; then
      if command -v typescript-language-server >/dev/null 2>&1; then
        run claude plugin enable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "typescript-lsp: typescript-language-server on PATH, plugin enabled"
      else
        run claude plugin disable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'npm install -g typescript-language-server typescript' || true
        if command -v typescript-language-server >/dev/null 2>&1; then
          run claude plugin enable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "typescript-lsp: typescript-language-server installed and enabled"
        else
          warn "typescript-lsp: typescript-language-server is absent; plugin left disabled (install missing: npm install -g typescript-language-server typescript)"
          missing_lsp_binaries+=("typescript-lsp: install with: npm install -g typescript-language-server typescript")
        fi
      fi
    else
      warn "typescript-lsp@claude-plugins-official install failed"
    fi

    # pyright-lsp -> pyright-langserver (python)
    if run claude plugin install pyright-lsp@claude-plugins-official --yes --scope user; then
      if command -v pyright-langserver >/dev/null 2>&1; then
        run claude plugin enable pyright-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "pyright-lsp: pyright-langserver on PATH, plugin enabled"
      else
        run claude plugin disable pyright-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'npm install -g pyright' || true
        if command -v pyright-langserver >/dev/null 2>&1; then
          run claude plugin enable pyright-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "pyright-lsp: pyright-langserver installed and enabled"
        else
          warn "pyright-lsp: pyright-langserver is absent; plugin left disabled (install missing: npm install -g pyright)"
          missing_lsp_binaries+=("pyright-lsp: install with: npm install -g pyright")
        fi
      fi
    else
      warn "pyright-lsp@claude-plugins-official install failed"
    fi

    # kotlin-lsp -> kotlin-lsp (kotlin)
    if run claude plugin install kotlin-lsp@claude-plugins-official --yes --scope user; then
      if command -v kotlin-lsp >/dev/null 2>&1; then
        run claude plugin enable kotlin-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "kotlin-lsp: kotlin-lsp on PATH, plugin enabled"
      else
        run claude plugin disable kotlin-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'brew install kotlin-lsp' || true
        if command -v kotlin-lsp >/dev/null 2>&1; then
          run claude plugin enable kotlin-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "kotlin-lsp: kotlin-lsp installed and enabled"
        else
          warn "kotlin-lsp: kotlin-lsp is absent; plugin left disabled (install missing: brew install kotlin-lsp)"
          missing_lsp_binaries+=("kotlin-lsp: install with: brew install kotlin-lsp")
        fi
      fi
    else
      warn "kotlin-lsp@claude-plugins-official install failed"
    fi

    # jdtls-lsp -> jdtls (java)
    if run claude plugin install jdtls-lsp@claude-plugins-official --yes --scope user; then
      if command -v jdtls >/dev/null 2>&1; then
        run claude plugin enable jdtls-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "jdtls-lsp: jdtls on PATH, plugin enabled"
      else
        run claude plugin disable jdtls-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'brew install jdtls' || true
        if command -v jdtls >/dev/null 2>&1; then
          run claude plugin enable jdtls-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "jdtls-lsp: jdtls installed and enabled"
        else
          warn "jdtls-lsp: jdtls is absent; plugin left disabled (install missing: brew install jdtls)"
          missing_lsp_binaries+=("jdtls-lsp: install with: brew install jdtls")
        fi
      fi
    else
      warn "jdtls-lsp@claude-plugins-official install failed"
    fi

    # ruby-lsp -> ruby-lsp (ruby)
    if run claude plugin install ruby-lsp@claude-plugins-official --yes --scope user; then
      if command -v ruby-lsp >/dev/null 2>&1; then
        run claude plugin enable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "ruby-lsp: ruby-lsp on PATH, plugin enabled"
      else
        run claude plugin disable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'gem install ruby-lsp' || true
        if command -v ruby-lsp >/dev/null 2>&1; then
          run claude plugin enable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "ruby-lsp: ruby-lsp installed and enabled"
        else
          warn "ruby-lsp: ruby-lsp is absent; plugin left disabled (install missing: gem install ruby-lsp)"
          missing_lsp_binaries+=("ruby-lsp: install with: gem install ruby-lsp")
        fi
      fi
    else
      warn "ruby-lsp@claude-plugins-official install failed"
    fi

    # gopls-lsp -> gopls (go)
    if run claude plugin install gopls-lsp@claude-plugins-official --yes --scope user; then
      if command -v gopls >/dev/null 2>&1; then
        run claude plugin enable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "gopls-lsp: gopls on PATH, plugin enabled"
      else
        run claude plugin disable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'go install golang.org/x/tools/gopls@latest' || true
        if command -v gopls >/dev/null 2>&1; then
          run claude plugin enable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "gopls-lsp: gopls installed and enabled"
        else
          warn "gopls-lsp: gopls is absent; plugin left disabled (install missing: go install golang.org/x/tools/gopls@latest)"
          missing_lsp_binaries+=("gopls-lsp: install with: go install golang.org/x/tools/gopls@latest")
        fi
      fi
    else
      warn "gopls-lsp@claude-plugins-official install failed"
    fi

    # rust-analyzer-lsp -> rust-analyzer (rust)
    if run claude plugin install rust-analyzer-lsp@claude-plugins-official --yes --scope user; then
      if command -v rust-analyzer >/dev/null 2>&1; then
        run claude plugin enable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "rust-analyzer-lsp: rust-analyzer on PATH, plugin enabled"
      else
        run claude plugin disable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'rustup component add rust-analyzer' || true
        if command -v rust-analyzer >/dev/null 2>&1; then
          run claude plugin enable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "rust-analyzer-lsp: rust-analyzer installed and enabled"
        else
          warn "rust-analyzer-lsp: rust-analyzer is absent; plugin left disabled (install missing: rustup component add rust-analyzer)"
          missing_lsp_binaries+=("rust-analyzer-lsp: install with: rustup component add rust-analyzer")
        fi
      fi
    else
      warn "rust-analyzer-lsp@claude-plugins-official install failed"
    fi

    # lua-lsp -> lua-language-server (lua)
    if run claude plugin install lua-lsp@claude-plugins-official --yes --scope user; then
      if command -v lua-language-server >/dev/null 2>&1; then
        run claude plugin enable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "lua-lsp: lua-language-server on PATH, plugin enabled"
      else
        run claude plugin disable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'brew install lua-language-server' || true
        if command -v lua-language-server >/dev/null 2>&1; then
          run claude plugin enable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "lua-lsp: lua-language-server installed and enabled"
        else
          warn "lua-lsp: lua-language-server is absent; plugin left disabled (install missing: brew install lua-language-server)"
          missing_lsp_binaries+=("lua-lsp: install with: brew install lua-language-server")
        fi
      fi
    else
      warn "lua-lsp@claude-plugins-official install failed"
    fi

    # clangd-lsp -> clangd (c, cpp)
    if run claude plugin install clangd-lsp@claude-plugins-official --yes --scope user; then
      if command -v clangd >/dev/null 2>&1; then
        run claude plugin enable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "clangd-lsp: clangd on PATH, plugin enabled"
      else
        run claude plugin disable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'brew install llvm' || true
        if command -v clangd >/dev/null 2>&1; then
          run claude plugin enable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "clangd-lsp: clangd installed and enabled"
        else
          warn "clangd-lsp: clangd is absent; plugin left disabled (install missing: brew install llvm)"
          missing_lsp_binaries+=("clangd-lsp: install with: brew install llvm")
        fi
      fi
    else
      warn "clangd-lsp@claude-plugins-official install failed"
    fi

    # php-lsp -> intelephense (php)
    if run claude plugin install php-lsp@claude-plugins-official --yes --scope user; then
      if command -v intelephense >/dev/null 2>&1; then
        run claude plugin enable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "php-lsp: intelephense on PATH, plugin enabled"
      else
        run claude plugin disable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'npm install -g intelephense' || true
        if command -v intelephense >/dev/null 2>&1; then
          run claude plugin enable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "php-lsp: intelephense installed and enabled"
        else
          warn "php-lsp: intelephense is absent; plugin left disabled (install missing: npm install -g intelephense)"
          missing_lsp_binaries+=("php-lsp: install with: npm install -g intelephense")
        fi
      fi
    else
      warn "php-lsp@claude-plugins-official install failed"
    fi

    # csharp-lsp -> csharp-ls (csharp)
    if run claude plugin install csharp-lsp@claude-plugins-official --yes --scope user; then
      if command -v csharp-ls >/dev/null 2>&1; then
        run claude plugin enable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "csharp-lsp: csharp-ls on PATH, plugin enabled"
      else
        run claude plugin disable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'dotnet tool install --global csharp-ls' || true
        if command -v csharp-ls >/dev/null 2>&1; then
          run claude plugin enable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "csharp-lsp: csharp-ls installed and enabled"
        else
          warn "csharp-lsp: csharp-ls is absent; plugin left disabled (install missing: dotnet tool install --global csharp-ls)"
          missing_lsp_binaries+=("csharp-lsp: install with: dotnet tool install --global csharp-ls")
        fi
      fi
    else
      warn "csharp-lsp@claude-plugins-official install failed"
    fi

    # swift-lsp -> sourcekit-lsp (swift)
    if run claude plugin install swift-lsp@claude-plugins-official --yes --scope user; then
      if command -v sourcekit-lsp >/dev/null 2>&1; then
        run claude plugin enable swift-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "swift-lsp: sourcekit-lsp on PATH, plugin enabled"
      else
        run claude plugin disable swift-lsp@claude-plugins-official >/dev/null 2>&1 || true
        warn "swift-lsp: sourcekit-lsp is absent and ships with its platform toolchain; plugin left disabled"
        missing_lsp_binaries+=("swift-lsp: missing from toolchain (no install command)")
      fi
    else
      warn "swift-lsp@claude-plugins-official install failed"
    fi

    # liquid-lsp -> shopify (liquid)
    if run claude plugin install liquid-lsp@claude-plugins-official --yes --scope user; then
      if command -v shopify >/dev/null 2>&1; then
        run claude plugin enable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "liquid-lsp: shopify on PATH, plugin enabled (was disabled in registry)"
      else
        run claude plugin disable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
        run_sh 'npm install -g @shopify/cli' || true
        if command -v shopify >/dev/null 2>&1; then
          run claude plugin enable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "liquid-lsp: shopify installed and enabled"
        else
          warn "liquid-lsp: shopify is absent; plugin left disabled (install missing: npm install -g @shopify/cli)"
          missing_lsp_binaries+=("liquid-lsp: install with: npm install -g @shopify/cli")
        fi
      fi
    else
      warn "liquid-lsp@claude-plugins-official install failed"
    fi
  else
    log "language servers skipped (--no-lsp)"
  fi

  # ---------------------------------------------------------------
  # 4. MCP servers, registered for Claude Code at user scope.
  #
  # The registry is authoritative: servers with `surfaces: []` or
  # `enabled: false` are removed. Others are ensured. Plugin-provided
  # servers (plugin:*:*) and hand-added ones are left untouched.
  # ---------------------------------------------------------------
  if [ "${DO_MCP}" = 1 ]; then
    log "MCP servers"
    run claude mcp remove --scope user memory >/dev/null 2>&1 || true
    log "memory: removed (retired in the registry)"
    run claude mcp remove --scope user vuetify >/dev/null 2>&1 || true
    log "vuetify: removed (retired in the registry)"
    run claude mcp remove --scope user knowledge >/dev/null 2>&1 || true
    log "knowledge: removed (retired in the registry)"


    # Self-hosted long-term memory. Replaces the retired knowledge
    # base; see docs/MEMORY.md.

    # Read-only cluster diagnostics through the server's own
    # ClusterRole.

    # kubernetes reaches the cluster via a kubectl port-forward of the
    # ClusterIP service kubernetes-mcp-server.agents-system (no ingress route exists for it).
    if ! curl -s -o /dev/null --max-time 1 "http://127.0.0.1:18080/" 2>/dev/null; then
      warn "kubernetes: starting kubectl port-forward svc/kubernetes-mcp-server 18080:8080"
      kubectl port-forward -n "agents-system" svc/kubernetes-mcp-server 18080:8080 >/dev/null 2>&1 &
      sleep 2
      curl -s -o /dev/null --max-time 1 "http://127.0.0.1:18080/" >/dev/null 2>&1 || warn "kubernetes: port-forward did not come up; registration may fail"
    fi
    if command -v claude >/dev/null 2>&1; then
      run claude mcp remove --scope user kubernetes >/dev/null 2>&1 || true
      if run_redacted "claude mcp add kubernetes" claude mcp add --scope user kubernetes --transport http http://127.0.0.1:18080/mcp; then
        ok "kubernetes registered (claude)"
      else
        fail "kubernetes registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove kubernetes >/dev/null 2>&1 || true
      if run_redacted "codex mcp add kubernetes" codex mcp add kubernetes --url http://127.0.0.1:18080/mcp; then
        ok "kubernetes registered (codex)"
      else
        fail "kubernetes registration failed (codex)"
      fi
    fi

    # Vuetify component API.
    # Hosted: untrusted data provider. Output is context, never instruction.

    # Browser automation.
    if ! command -v npx >/dev/null 2>&1; then
      command -v npx >/dev/null 2>&1 \
        || warn "playwright: npx is not on PATH; skipping"
    fi
    if command -v claude >/dev/null 2>&1; then
      run claude mcp remove --scope user playwright >/dev/null 2>&1 || true
      if run_redacted "claude mcp add playwright" claude mcp add --scope user playwright -- npx -y @playwright/mcp@latest --headless --browser chromium; then
        ok "playwright registered (claude)"
      else
        fail "playwright registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove playwright >/dev/null 2>&1 || true
      if run_redacted "codex mcp add playwright" codex mcp add playwright -- npx -y @playwright/mcp@latest --headless --browser chromium; then
        ok "playwright registered (codex)"
      else
        fail "playwright registration failed (codex)"
      fi
    fi

    # Diagram authoring and export.
    if ! command -v drawio-mcp >/dev/null 2>&1; then
      run_sh 'npm install -g @drawio/mcp@latest' || true
      command -v drawio-mcp >/dev/null 2>&1 \
        || warn "drawio: drawio-mcp is not on PATH; skipping"
    fi
    if command -v claude >/dev/null 2>&1; then
      run claude mcp remove --scope user drawio >/dev/null 2>&1 || true
      if run_redacted "claude mcp add drawio" claude mcp add --scope user drawio -- drawio-mcp; then
        ok "drawio registered (claude)"
      else
        fail "drawio registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove drawio >/dev/null 2>&1 || true
      if run_redacted "codex mcp add drawio" codex mcp add drawio -- drawio-mcp; then
        ok "drawio registered (codex)"
      else
        fail "drawio registration failed (codex)"
      fi
    fi

    # Self-hosted Overleaf — pull, push, compile, review comments.
    if ! command -v olcli-mcp >/dev/null 2>&1; then
      command -v olcli-mcp >/dev/null 2>&1 \
        || warn "overleaf: olcli-mcp is not on PATH; skipping"
    fi
    if command -v claude >/dev/null 2>&1; then
      run claude mcp remove --scope user overleaf >/dev/null 2>&1 || true
      if run_redacted "claude mcp add overleaf" claude mcp add --scope user overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OVERLEAF_SESSION="${OVERLEAF_SESSION}" -- olcli-mcp; then
        ok "overleaf registered (claude)"
      else
        fail "overleaf registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove overleaf >/dev/null 2>&1 || true
      if run_redacted "codex mcp add overleaf" codex mcp add overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OVERLEAF_SESSION="${OVERLEAF_SESSION}" -- olcli-mcp; then
        ok "overleaf registered (codex)"
      else
        fail "overleaf registration failed (codex)"
      fi
    fi

    # Local Hermes reads its MCP servers from ~/.hermes/config.yaml.
    # `hermes mcp add` is interactive (probes + prompts), so the setup
    # script merges the generated workstation block via a helper instead.
    if command -v hermes >/dev/null 2>&1; then
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "would merge local Hermes MCP servers"
      else
        uv run --directory "${KIT_ROOT}" python \
          scripts/hermes-merge-mcp.py "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml"
      fi
    else
      warn "hermes not on PATH; local Hermes MCP config not merged"
    fi
    # Verify the VALUE, not the exit codes: ask Claude what it
    # actually has. Check expected servers are present, and report
    # any unexpected ones (but leave plugin-provided and hand-added).
    if [ "${CHECK_ONLY}" != 1 ]; then
      registered=$(claude mcp list 2>/dev/null || true)
      # Check all expected servers are registered
      for want in \
        kubernetes \
        playwright \
        drawio \
        overleaf \
        ; do
        case "${registered}" in
          *"${want}"*) ;;
          *) warn "MCP server ${want} is not in \`claude mcp list\` output" ;;
        esac
      done
      # Report unknown servers (but ignore plugin-provided and hand-added ones)
      echo "${registered}" | grep -oE "\b[a-z0-9_-]+\b(?=:)" | sort -u | while read -r found; do
        case "${found}" in
          memory) ;;
          kubernetes) ;;
          vuetify) ;;
          playwright) ;;
          drawio) ;;
          overleaf) ;;
          plugin:*|idea|rubymine) ;;
          *) warn "unknown MCP server ${found} -- hand-added or from a removed registry entry?" ;;
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
# 7. Summary: what changed and what still needs attention.
# -----------------------------------------------------------------

log "Summary of findings:"

# Report newly installed binaries that need a new shell
if [ "${#new_binaries[@]}" -gt 0 ]; then
  log "Binaries installed during this run (may need a new shell):"
  for bin in "${new_binaries[@]}"; do
    log "  - ${bin}"
  done
  log ""
fi

# Report MCP servers skipped due to missing credentials
if [ "${#skipped_mcp_servers[@]}" -gt 0 ]; then
  log "MCP servers not registered (missing credentials):"
  for entry in "${skipped_mcp_servers[@]}"; do
    log "  - ${entry}"
  done
  log ""
fi

# Report language servers with missing binaries
if [ "${#missing_lsp_binaries[@]}" -gt 0 ]; then
  log "Language servers with missing binaries:"
  for entry in "${missing_lsp_binaries[@]}"; do
    log "  - ${entry}"
  done
  log ""
fi

# Report plugins left disabled
if [ "${#disabled_plugins[@]}" -gt 0 ]; then
  log "Plugins left disabled:"
  for entry in "${disabled_plugins[@]}"; do
    log "  - ${entry}"
  done
  log ""
fi

# Report plugins with drifted commits
if [ "${#plugin_drift[@]}" -gt 0 ]; then
  log "Plugins with drifted commits:"
  for entry in "${plugin_drift[@]}"; do
    log "  - ${entry}"
  done
  log ""
fi

# Final summary
if [ "${failures}" = 0 ] && [ "${warnings}" = 0 ] \
   && [ "${#skipped_mcp_servers[@]}" = 0 ] && [ "${#missing_lsp_binaries[@]}" = 0 ] \
   && [ "${#disabled_plugins[@]}" = 0 ] && [ "${#plugin_drift[@]}" = 0 ]; then
  log "Setup complete: everything is ready"
else
  log "Setup summary: ${failures} failure(s), ${warnings} warning(s)"
fi

[ "${failures}" = 0 ]
