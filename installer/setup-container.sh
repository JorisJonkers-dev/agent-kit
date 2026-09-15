#!/usr/bin/env bash
# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.
#
# Installs the agents image's tools at their registry pins. Root, build time, no secrets.
#
# Usage:
#   ./setup-container.sh           install everything, then verify (root)
#   ./setup-container.sh --check   verify only: every tool present at its pin

set -euo pipefail

CHECK_ONLY=0
case "${1:-}" in
  --check) CHECK_ONLY=1 ;;
  "") ;;
  --help|-h) sed -n '2,8p' "$0"; exit 0 ;;
  *) echo "unknown option: $1" >&2; exit 64 ;;
esac

failures=0
log()  { printf 'setup-container: %s\n' "$*"; }
ok()   { printf 'setup-container:   ok    %s\n' "$*"; }
fail() { printf 'setup-container:   FAIL  %s\n' "$*" >&2; failures=$((failures + 1)); }

export DEBIAN_FRONTEND=noninteractive
export UV_TOOL_DIR='/opt/uv/tools'
export UV_TOOL_BIN_DIR='/usr/local/bin'
export UV_PYTHON_INSTALL_DIR='/opt/uv/python'
export PLAYWRIGHT_BROWSERS_PATH='/ms-playwright'

case "$(uname -m)" in
  x86_64|amd64) DEB_ARCH=amd64 GNU_ARCH=x86_64 ;;
  aarch64|arm64) DEB_ARCH=arm64 GNU_ARCH=aarch64 ;;
  *) echo "setup-container: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
export DEB_ARCH GNU_ARCH

install_tool() {
  log "install $1 $2"
  VERSION="$2" bash -euo pipefail -c "$3"
}

# Checks the reported version, so a stale binary earlier on PATH fails.
check_tool() {
  local binary="$1" version="${2#v}" version_command="$3" output
  if ! command -v "${binary}" >/dev/null 2>&1; then
    fail "${binary}: not on PATH"
    return 0
  fi
  output="$(bash -c "${version_command}" 2>&1 || true)"
  if printf '%s\n' "${output}" | grep -Eq "(^|[^0-9.])${version//./\\.}([^0-9.]|$)"; then
    ok "${binary} ${version}"
  else
    fail "${binary}: expected ${version}, got: $(printf '%s\n' "${output}" | head -1)"
  fi
}

check_apt() {
  if dpkg -s "$1" >/dev/null 2>&1; then
    ok "apt $1"
  else
    fail "apt $1: not installed"
  fi
}

if [ "${CHECK_ONLY}" = 0 ]; then
  if [ "$(id -u)" != 0 ]; then
    echo "setup-container: must run as root (image build time); use --check otherwise" >&2
    exit 77
  fi

  log "apt repositories"
  apt-get update
  apt-get install -y --no-install-recommends ca-certificates curl gnupg
  # shellcheck source=/dev/null
  codename="$(. /etc/os-release && echo "${VERSION_CODENAME}")"
  curl -fsSL 'https://packages.adoptium.net/artifactory/api/gpg/key/public' | gpg --dearmor -o /usr/share/keyrings/adoptium.gpg
  echo "deb [signed-by=/usr/share/keyrings/adoptium.gpg] https://packages.adoptium.net/artifactory/deb ${codename} main" > /etc/apt/sources.list.d/adoptium.list
  apt-get update
  log "apt packages"
  apt-get install -y --no-install-recommends \
    bash \
    bat \
    bubblewrap \
    build-essential \
    clangd \
    fd-find \
    git \
    jq \
    less \
    openssh-client \
    procps \
    python3 \
    python3-pip \
    python3-venv \
    ripgrep \
    temurin-21-jdk \
    tini \
    tmux \
    unzip \
    xz-utils \
    zsh \
    ;

  # Single quotes on purpose: install_tool expands ${VERSION} per tool.
  # shellcheck disable=SC2016
  {
  install_tool node '22.23.2' 'curl -fsSL "https://nodejs.org/dist/v${VERSION}/node-v${VERSION}-linux-${DEB_ARCH/amd64/x64}.tar.xz" | tar -xJ -C /usr/local --strip-components=1 --exclude='\''*.md'\'' --exclude=LICENSE --exclude=share/doc'
  install_tool claude-code '2.1.272' 'npm install -g "@anthropic-ai/claude-code@${VERSION}"'
  install_tool codex '0.154.0' 'npm install -g "@openai/codex@${VERSION}"'
  install_tool uv '0.12.15' 'curl -fsSL "https://github.com/astral-sh/uv/releases/download/${VERSION}/uv-${GNU_ARCH}-unknown-linux-gnu.tar.gz" | tar -xz -C /usr/local/bin --strip-components=1 --wildcards '\''*/uv'\'' '\''*/uvx'\'''
  install_tool hermes-agent '0.19.0' 'uv tool install --python 3.13 "hermes-agent[mcp]==${VERSION}"'
  install_tool olcli '0.13.0' 'npm install -g "@aloth/olcli@${VERSION}"'
  install_tool gh 'v2.100.0' 'curl -fsSL "https://github.com/cli/cli/releases/download/${VERSION}/gh_${VERSION#v}_linux_${DEB_ARCH}.tar.gz" | tar -xz -C /usr/local --strip-components=1 "gh_${VERSION#v}_linux_${DEB_ARCH}/bin/gh"'
  install_tool go '1.27.1' 'curl -fsSL "https://go.dev/dl/go${VERSION}.linux-${DEB_ARCH}.tar.gz" | tar -xz -C /usr/local && ln -sf /usr/local/go/bin/go /usr/local/go/bin/gofmt /usr/local/bin/'
  install_tool mise 'v2026.9.9' 'curl -fsSL -o /usr/local/bin/mise "https://github.com/jdx/mise/releases/download/${VERSION}/mise-${VERSION}-linux-${DEB_ARCH/amd64/x64}" && chmod 755 /usr/local/bin/mise'
  install_tool ast-grep '0.45.3' 'npm install -g "@ast-grep/cli@${VERSION}"'
  install_tool typescript '5.9.3' 'npm install -g "typescript@${VERSION}"'
  install_tool github-mcp-server 'v1.12.1' 'case "${DEB_ARCH}" in amd64) a=x86_64 ;; arm64) a=arm64 ;; esac && curl -fsSL "https://github.com/github/github-mcp-server/releases/download/${VERSION}/github-mcp-server_Linux_${a}.tar.gz" | tar -xz -C /usr/local/bin github-mcp-server'
  install_tool serena '1.7.0' 'uv tool install --python 3.13 "serena-agent==${VERSION}"'
  install_tool playwright '0.0.81' 'npm install -g "@playwright/mcp@${VERSION}" && node "$(npm root -g)/@playwright/mcp/node_modules/playwright/cli.js" install --with-deps chromium'
  install_tool drawio '1.5.0' 'npm install -g "@drawio/mcp@${VERSION}"'
  install_tool typescript-lsp '6.0.0' 'npm install -g "typescript-language-server@${VERSION}"'
  install_tool pyright-lsp '1.1.414' 'npm install -g "pyright@${VERSION}"'
  install_tool gopls-lsp 'v0.23.0' 'GOBIN=/usr/local/bin GOPATH=/tmp/gopath GOCACHE=/tmp/gocache go install "golang.org/x/tools/gopls@${VERSION}" && GOPATH=/tmp/gopath go clean -modcache && rm -rf /tmp/gopath /tmp/gocache'
  install_tool php-lsp '1.18.5' 'npm install -g "intelephense@${VERSION}"'
  }

  [ ! -e '/opt/uv' ] || chmod -R a+rX '/opt/uv'
  [ ! -e '/ms-playwright' ] || chmod -R a+rX '/ms-playwright'
  npm cache clean --force >/dev/null 2>&1 || true
  rm -rf /var/lib/apt/lists/* /root/.cache /root/.npm /tmp/*
fi

log "verify"
check_apt ca-certificates
check_apt curl
check_apt gnupg
check_apt bash
check_apt bat
check_apt bubblewrap
check_apt build-essential
check_apt clangd
check_apt fd-find
check_apt git
check_apt jq
check_apt less
check_apt openssh-client
check_apt procps
check_apt python3
check_apt python3-pip
check_apt python3-venv
check_apt ripgrep
check_apt temurin-21-jdk
check_apt tini
check_apt tmux
check_apt unzip
check_apt xz-utils
check_apt zsh
check_tool node '22.23.2' 'node --version'
check_tool claude '2.1.272' 'claude --version'
check_tool codex '0.154.0' 'codex --version'
check_tool uv '0.12.15' 'uv --version'
check_tool hermes '0.19.0' 'hermes --version'
check_tool olcli '0.13.0' 'olcli --version'
check_tool gh 'v2.100.0' 'gh --version'
check_tool go '1.27.1' 'go version'
check_tool mise 'v2026.9.9' 'mise --version'
check_tool ast-grep '0.45.3' 'ast-grep --version'
check_tool tsserver '5.9.3' 'tsc --version'
check_tool github-mcp-server 'v1.12.1' 'github-mcp-server --version'
check_tool serena '1.7.0' 'serena --version'
check_tool playwright-mcp '0.0.81' 'npm ls -g @playwright/mcp'
check_tool drawio-mcp '1.5.0' 'npm ls -g @drawio/mcp'
check_tool typescript-language-server '6.0.0' 'typescript-language-server --version'
check_tool pyright-langserver '1.1.414' 'pyright --version'
check_tool gopls 'v0.23.0' 'gopls version'
check_tool intelephense '1.18.5' 'npm ls -g intelephense'

if [ "${failures}" != 0 ]; then
  log "${failures} check(s) failed"
  exit 1
fi
log "every tool is at its pinned version"
