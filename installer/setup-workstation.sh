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
#   ./setup-workstation.sh --no-profiles   only the primary Claude profile
#   ./setup-workstation.sh --no-prompt     never ask; report what needs a hand
#   ./setup-workstation.sh --cloud         a Claude Code cloud environment's
#                                          setup script (docs/CLOUD.md)
#   ./setup-workstation.sh --uninstall     remove what a PREVIOUS install.sh /
#                                          install-agents.sh install wrote,
#                                          then purge retired knowledge hooks,
#                                          remove the critique hooks and exit
#
# Run outside an agent-kit checkout (`curl | bash`), it fetches the kit
# bundle attached to its own GitHub release and re-runs the copy inside it.
# AGENT_KIT_SKILLS_BUNDLE_URL overrides where that bundle comes from.
#
# Secrets are read from the environment and never written here:
#   HINDSIGHT_API_TOKEN  -> the memory-api MCP server
#   MEMORY_PRIVATE_TOKEN  -> the memory-private MCP server
#   GRIMOIRE_API_TOKEN  -> the grimoire MCP server
#   OUTLINE_API_KEY  -> the outline MCP server
#   OVERLEAF_SESSION  -> the overleaf MCP server

set -uo pipefail

CHECK_ONLY=0
DO_LSP=1
DO_MCP=1
DO_PROFILES=1
CLOUD=0
UNINSTALL=0
PROMPT=1
PROMPT_TTY=/dev/tty
failures=0
warnings=0
todo_failures=()           # Every failure, listed again at the end
todo_warnings=()           # Every warning, listed again at the end

# Track what needs attention after the run
skipped_mcp_servers=()     # MCP servers skipped due to missing credentials
skipped_mcp_names=()       # Their names, so the registration check skips them
unpinned_markets=()        # Marketplaces not at their pin: their plugins are not installed
unshared_profile_paths=()  # Shared surfaces a secondary profile did not get
missing_lsp_binaries=()    # Language servers with missing binaries
disabled_plugins=()        # Plugins disabled (missing binary or on purpose)
missing_plugin_env=()      # Plugins whose required env vars are unset
plugin_drift=()            # Plugins whose commit drifted
new_binaries=()            # Binaries installed during this run

ORIG_ARGS=("$@")
while [ "$#" -gt 0 ]; do
  case "$1" in
    --check) CHECK_ONLY=1 ;;
    --no-lsp) DO_LSP=0 ;;
    --no-mcp) DO_MCP=0 ;;
    --no-profiles) DO_PROFILES=0 ;;
    --no-prompt) PROMPT=0 ;;
    --cloud) CLOUD=1; DO_PROFILES=0 ;;
    --uninstall) UNINSTALL=1 ;;
    --help|-h) sed -n '2,34p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
  shift
done

log()  { printf 'setup: %s\n' "$*"; }
ok()   { printf 'setup:   ok    %s\n' "$*"; }
warn() { printf 'setup:   warn  %s\n' "$*" >&2; warnings=$((warnings + 1)); todo_warnings+=("$*"); }
fail() { printf 'setup:   FAIL  %s\n' "$*" >&2; failures=$((failures + 1)); todo_failures+=("$*"); }

# Whether NixOS provides this binary: some copy on PATH resolves into the store.
nix_provides() {
  [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ] || return 1
  local path resolved
  while read -r path; do
    [ -n "${path}" ] || continue
    resolved=$(readlink -f "${path}" 2>/dev/null) || continue
    case "${resolved}" in "${AK_NIX_STORE:-/nix/store}/"*) return 0 ;; esac
  done < <(type -aP "$1" 2>/dev/null)
  return 1
}

# The first non-Nix copy of this binary ahead of any Nix copy on PATH: the
# one that actually runs instead of Nix's. A copy after the Nix one shadows
# nothing, so the search stops at the first store copy.
non_nix_copy() {
  local path resolved
  while read -r path; do
    [ -n "${path}" ] || continue
    resolved=$(readlink -f "${path}" 2>/dev/null) || continue
    case "${resolved}" in "${AK_NIX_STORE:-/nix/store}/"*) return 1 ;; esac
    printf '%s\n' "${path}"
    return 0
  done < <(type -aP "$1" 2>/dev/null)
  return 1
}

# Everything the run left for a person to do: failures first, each once.
print_todo() {
  local all=() unique=() entry seen i=0
  for entry in ${todo_failures[@]+"${todo_failures[@]}"}; do all+=("FAIL ${entry}"); done
  for entry in ${todo_warnings[@]+"${todo_warnings[@]}"}; do all+=("${entry}"); done
  for entry in ${all[@]+"${all[@]}"}; do
    for seen in ${unique[@]+"${unique[@]}"}; do
      [ "${seen}" = "${entry}" ] && continue 2
    done
    unique+=("${entry}")
  done
  if [ "${#unique[@]}" = 0 ]; then
    log "Nothing left to do."
    return 0
  fi
  log "Still to do (${#unique[@]}):"
  for entry in "${unique[@]}"; do
    i=$((i + 1))
    log "  ${i}. ${entry}"
  done
}

# Asks only with someone at a terminal: never under --check, --cloud, CI or --no-prompt.
interactive() {
  [ "${PROMPT}" = 1 ] && [ "${CHECK_ONLY}" != 1 ] && [ "${CLOUD}" != 1 ] && [ -t 1 ] \
    && { : </dev/tty; } 2>/dev/null
}

# Tokens and keys first: everything after this reads them. In a terminal
# agent-kit-keys signs you in, mints what is missing and stores it; either way
# env.sh is rewritten and loaded, so this run registers what needs them.
tokens_and_keys() {
  local table="${KIT_ROOT}/registry/generated/workstation-secrets.json"
  local helper="${KIT_ROOT}/scripts/agent-kit-keys.py"
  local var state
  if [ "${CLOUD}" = 1 ]; then
    log "tokens and keys: the cloud session brings its own"
    return 0
  fi
  log "tokens and keys"
  if interactive; then
    python3 "${helper}" --table "${table}" ensure <"${PROMPT_TTY}" \
      || warn "tokens and keys: the walk-through stopped early; run setup again to continue"
  elif [ "${CHECK_ONLY}" != 1 ]; then
    python3 "${helper}" --table "${table}" write-env \
      || warn "tokens and keys: could not write ~/.config/agent-kit/env.sh"
  fi
  while read -r var state; do
    case "${var} ${state}" in
      "shell missing")
        warn "your shell does not load ~/.config/agent-kit/env.sh; run setup in a terminal to add it" ;;
      "MISTRAL_STUDY_API_KEY missing")
        warn "MISTRAL_STUDY_API_KEY is missing; run  vibe-study --login  in a terminal" ;;
      "MISTRAL_STUDY_API_KEY invalid")
        warn "MISTRAL_STUDY_API_KEY is refused by its host; run  vibe-study --login  in a terminal again" ;;
      *" missing") warn "${var} is missing; run setup in a terminal and it gets it for you" ;;
      *" invalid") warn "${var} is refused by its host; run setup in a terminal to replace it" ;;
      *" unchecked") log "${var}: its host did not answer, so it was not checked" ;;
      *" ok") ok "${var}" ;;
    esac
  done < <(python3 "${helper}" --table "${table}" status)
  if [ -f "${HOME}/.config/agent-kit/env.sh" ]; then
    # shellcheck disable=SC1091
    . "${HOME}/.config/agent-kit/env.sh"
    # Its vibe and memory-private wrappers are for your shell, not for this run.
    unset -f vibe memory-private 2>/dev/null || true
  fi
}

# -----------------------------------------------------------------
# The packages proxy
# -----------------------------------------------------------------

# pp_resolve_key prints the proxy key (always captured, never logged),
# in order: PACKAGES_PROXY_TOKEN, the vault secret, the stored token file.
# The last resort is a prompt, and only in an interactive run.
pp_resolve_key() {
  local root="$1"
  local token="${XDG_CONFIG_HOME:-${root}/.config}/jorisjonkers/packages-token"
  local key=""
  if [ -n "${PACKAGES_PROXY_TOKEN:-}" ]; then
    printf '%s' "${PACKAGES_PROXY_TOKEN}"
    return 0
  fi
  if command -v vault >/dev/null 2>&1 && vault token lookup >/dev/null 2>&1; then
    key="$(vault kv get -mount=secret -field=workstation packages-system/keys 2>/dev/null)"
    if [ -n "${key}" ]; then
      printf '%s' "${key}"
      return 0
    fi
  fi
  if [ -s "${token}" ]; then
    printf '%s' "$(cat "${token}" 2>/dev/null)"
    return 0
  fi
  if interactive; then
    printf 'packages proxy key: ' >&2
    read -rs key <"${PROMPT_TTY}"
    printf '\n' >&2
    if [ -n "${key}" ]; then
      printf '%s' "${key}"
      return 0
    fi
  fi
  return 1
}

# pp_store_key writes the key to the token file, atomically and 0600: a
# temp file under umask 077 in the same directory, then a rename. A file
# that already holds the key is only chmodded back to 0600.
pp_store_key() {
  local root="$1" key="$2"
  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"
  local dest="${config_root}/jorisjonkers/packages-token"
  case "${config_root}" in
    "${root}"|"${root}"/*) ;;
    *)
      warn "packages proxy: the config root is not under ${root}, so the key is not stored"
      return 1 ;;
  esac
  if [ -f "${dest}" ] && [ "$(cat "${dest}" 2>/dev/null)" = "${key}" ]; then
    chmod 600 "${dest}" 2>/dev/null || true
    return 0
  fi
  (
    umask 077
    mkdir -p "$(dirname "${dest}")"
    local tmp
    tmp="$(mktemp "${dest}.tmp.XXXXXX")"
    printf '%s' "${key}" >"${tmp}"
    mv -f "${tmp}" "${dest}"
  )
  chmod 600 "${dest}"
}

# pp_write_netrc owns exactly the proxy machine entry of ROOT/.netrc.
# netrc has no comment markers, so there is no marked block to own:
# an existing proxy entry, single- or multi-line, is replaced where it
# stands (the entry runs to the next machine/default/token line) and
# every other line keeps as the user wrote it. The first change is
# backed up once to .netrc.bak-agent-kit, the file ends up 0600, and
# the key travels in shell memory only, never on argv.
pp_write_netrc() {
  local root="$1" key="$2"
  local netrc="${root}/.netrc" host="packages.jorisjonkers.dev"
  local entry="machine ${host} login packages password ${key}"
  local text="" line="" first="" second="" skip=0 found=0 tmp
  local nl=$'\n'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the ${host} machine entry in ${netrc}"
    return 0
  fi
  if [ -f "${netrc}" ]; then
    while IFS= read -r line || [ -n "${line}" ]; do
      read -r first second rest <<< "${line}"
      if [ "${skip}" = 1 ]; then
        case "${first}" in
          login|password|account|force) continue ;;
        esac
        skip=0
      fi
      if [ "${first}" = machine ] && [ "${second}" = "${host}" ]; then
        text="${text}${entry}${nl}"
        skip=1
        found=1
        continue
      fi
      text="${text}${line}${nl}"
    done < "${netrc}"
  fi
  if [ "${found}" = 0 ]; then
    text="${text}${entry}${nl}"
  fi
  if [ -f "${netrc}" ] && [ "$(cat "${netrc}")" = "${text%"${nl}"}" ]; then
    chmod 600 "${netrc}" 2>/dev/null || true
    log "packages proxy: ${netrc} is already set up"
    return 0
  fi
  if [ -f "${netrc}" ] && [ ! -f "${netrc}.bak-agent-kit" ]; then
    cp "${netrc}" "${netrc}.bak-agent-kit"
  fi
  (
    umask 077
    tmp="$(mktemp "${netrc}.tmp.XXXXXX")"
    printf '%s\n' "${text%"${nl}"}" > "${tmp}"
    mv -f "${tmp}" "${netrc}"
  )
  chmod 600 "${netrc}"
  log "packages proxy: ${netrc} is set up"
}

# pp_write_go points go at the proxy -- but only when go is on PATH,
# and only into this root's env file: GOENV names it (ROOT/.config/go/env
# by default), and a GOENV outside the root is refused outright. No
# GOSUMDB and no ,direct: the proxy is the only source. CHECK_ONLY
# never calls go.
pp_write_go() {
  local root="$1"
  local goenv="${GOENV:-${root}/.config/go/env}"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would point go at the packages proxy through ${goenv}"
    return 0
  fi
  if ! command -v go >/dev/null 2>&1; then
    log "packages proxy: go is not on PATH, its env stays as it is"
    return 0
  fi
  case "${goenv}" in
    "${root}"|"${root}"/*) ;;
    *)
      warn "packages proxy: GOENV is not under ${root}, go env left alone"
      return 0 ;;
  esac
  mkdir -p "$(dirname "${goenv}")"
  if ! GOENV="${goenv}" go env -w "GOPROXY=https://packages.jorisjonkers.dev/go" "GOPRIVATE=github.com/JorisJonkers-dev/*"; then
    return 1
  fi
  ok "packages proxy: go env points at the proxy (${goenv})"
}

# pp_write_npmrc sets the proxy registry and its token in ONE marked
# block of ROOT/.npmrc, through the merge helper: everything outside the
# block keeps as the user wrote it (the first-party scope and its GitHub
# token line included), a registry= line outside the block refuses the
# merge outright, and the file ends up 0600. The token rides stdin,
# never argv or a log line.
pp_write_npmrc() {
  local root="$1" key="$2"
  local npmrc="${root}/.npmrc"
  local helper="${KIT_ROOT}/scripts/package-proxy-merge.py"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the packages registry and token in ${npmrc}"
    return 0
  fi
  printf 'registry=https://packages.jorisjonkers.dev/npm/\n//packages.jorisjonkers.dev/npm/:_authToken=%s\n' "${key}" \
    | python3 "${helper}" "${npmrc}" packages-proxy --mode 600 --refuse '^registry=' >/dev/null
  case $? in
    0) ok "packages proxy: ${npmrc} is set up" ;;
    3) warn "packages proxy: ${npmrc} has its own registry= line, nothing written" ;;
    *) return 1 ;;
  esac
}

# pp_write_uv points uv at the proxy through ONE marked block of the uv
# config: a default index served by the proxy, capped at a week of
# package age. A user exclude-newer outside the block refuses the
# merge, and everything else in the file keeps as the user wrote it.
# The config root is XDG_CONFIG_HOME when it sits under the root, else
# ROOT/.config -- a config root outside the root is never written.
pp_write_uv() {
  local root="$1"
  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"
  local uv_toml="${config_root}/uv/uv.toml"
  local helper="${KIT_ROOT}/scripts/package-proxy-merge.py"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the packages pypi index in ${uv_toml}"
    return 0
  fi
  case "${config_root}" in
    "${root}"|"${root}"/*) ;;
    *)
      warn "packages proxy: the config root is not under ${root}, uv is left alone"
      return 0 ;;
  esac
  printf '[[index]]\nurl = "https://packages.jorisjonkers.dev/pypi/simple/"\ndefault = true\nexclude-newer = "7 days"\n' \
    | python3 "${helper}" "${uv_toml}" packages-proxy --validate toml \
      --refuse '^exclude-newer' >/dev/null
  case $? in
    0) ok "packages proxy: ${uv_toml} is set up" ;;
    3) warn "packages proxy: ${uv_toml} has its own exclude-newer, nothing written" ;;
    *) return 1 ;;
  esac
}

# pp_write_pip points pip at the proxy through ONE marked block of
# pip.conf: index-url under [global]. A [global] section the user
# already wrote must not come through twice, so the helper is asked on
# a throwaway copy (its --refuse reads the copy without this block)
# whether one stands outside the block; when it does the merged block
# carries the bare line, which [global] above already owns. A user
# index-url outside the block refuses the merge outright.
pp_write_pip() {
  local root="$1"
  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"
  local pip_conf="${config_root}/pip/pip.conf" body probe
  local helper="${KIT_ROOT}/scripts/package-proxy-merge.py"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the packages pypi index in ${pip_conf}"
    return 0
  fi
  case "${config_root}" in
    "${root}"|"${root}"/*) ;;
    *)
      warn "packages proxy: the config root is not under ${root}, pip is left alone"
      return 0 ;;
  esac
  body=$'[global]\nindex-url = https://packages.jorisjonkers.dev/pypi/simple/'
  if [ -f "${pip_conf}" ]; then
    probe="$(mktemp "${pip_conf}.probe.XXXXXX")"
    cp "${pip_conf}" "${probe}"
    python3 "${helper}" "${probe}" packages-proxy --remove \
      --refuse '^\[global\]' >/dev/null 2>&1 </dev/null
    case $? in
      3) body=$'index-url = https://packages.jorisjonkers.dev/pypi/simple/' ;;
    esac
    rm -f "${probe}" "${probe}.bak-agent-kit"
  fi
  printf '%s\n' "${body}" \
    | python3 "${helper}" "${pip_conf}" packages-proxy --validate ini \
      --refuse '^index-url' >/dev/null
  case $? in
    0) ok "packages proxy: ${pip_conf} is set up" ;;
    3) warn "packages proxy: ${pip_conf} has its own index-url, nothing written" ;;
    *) return 1 ;;
  esac
}

# pp_write_cargo points cargo at the proxy through TWO marked blocks of
# the cargo home: the registry replacement in config.toml (no key in
# it, so no --mode) and the login token in credentials.toml (written
# 0600, its key riding stdin). CARGO_HOME counts only when it is set
# AND under the root; anything else falls back to ROOT/.cargo, so a
# writer never writes outside its root. A table this step must own
# that already stands outside the block refuses that file's merge --
# and the other file still gets its block.
pp_write_cargo() {
  local root="$1" key="$2"
  local cargo_home="${root}/.cargo"
  case "${CARGO_HOME:-}" in
    "") ;;
    "${root}"|"${root}"/*) cargo_home="${CARGO_HOME}" ;;
    *) log "packages proxy: CARGO_HOME is not under ${root}, cargo is configured in ${cargo_home}" ;;
  esac
  local cargo_config="${cargo_home}/config.toml"
  local credentials="${cargo_home}/credentials.toml"
  local helper="${KIT_ROOT}/scripts/package-proxy-merge.py"
  local guard='^\[(source\.(crates-io|jorisjonkers-packages)|'\
$'registries\.jorisjonkers-packages|registry)\]'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the cargo registry in ${cargo_config} and its token in ${credentials}"
    return 0
  fi
  {
    printf '[source.crates-io]\nreplace-with = "jorisjonkers-packages"\n\n'
    printf '[source.jorisjonkers-packages]\nregistry = "sparse+https://packages.jorisjonkers.dev/cargo/"\n\n'
    printf '[registries.jorisjonkers-packages]\nindex = "sparse+https://packages.jorisjonkers.dev/cargo/"\n\n'
    printf '[registry]\nglobal-credential-providers = ["cargo:token"]\n'
    printf 'global-min-publish-age = "7 days"\n'
  } | python3 "${helper}" "${cargo_config}" packages-proxy --validate toml \
    --refuse "${guard}" >/dev/null
  case $? in
    0) ok "packages proxy: ${cargo_config} is set up" ;;
    3) warn "packages proxy: ${cargo_config} has its own cargo registry tables, nothing written" ;;
    *) return 1 ;;
  esac
  printf '[registries.jorisjonkers-packages]\ntoken = "Bearer %s"\n' "${key}" \
    | python3 "${helper}" "${credentials}" packages-proxy --validate toml --mode 600 \
      --refuse '^\[registries\.jorisjonkers-packages\]' >/dev/null
  case $? in
    0) ok "packages proxy: ${credentials} is set up" ;;
    3) warn "packages proxy: ${credentials} has its own registry token, nothing written" ;;
    *) return 1 ;;
  esac
}

# pp_write_gradle writes Gradle's packages-proxy init script -- copied
# from the estate's setup-java-gradle action, with only the token
# source changed -- plus the packagesProxyToken Gradle property and the
# wrapper proxy credentials that carry the key. The init script holds no
# key (its Kotlin reads the token from the property or the environment at
# build time), so it is written 0644 through a quoted heredoc (no shell
# expansion inside it); gradle.properties holds the key and is written 0600
# through the merge helper, its key riding stdin -- a packagesProxyToken=,
# systemProp.gradle.wrapperUser= or systemProp.gradle.wrapperPassword= line
# outside the block refusing the merge outright. GRADLE_USER_HOME counts only
# when it is set AND under the root; anything else falls back to ROOT/.gradle,
# so the writer never writes outside its root.
pp_write_gradle() {
  local root="$1" key="$2"
  local gradle_home="${root}/.gradle"
  case "${GRADLE_USER_HOME:-}" in
    "") ;;
    "${root}"|"${root}"/*) gradle_home="${GRADLE_USER_HOME}" ;;
    *) log "packages proxy: GRADLE_USER_HOME is not under ${root}, gradle is configured in ${gradle_home}" ;;
  esac
  local init_script="${gradle_home}/init.d/packages-proxy.init.gradle.kts"
  local gradle_props="${gradle_home}/gradle.properties"
  local helper="${KIT_ROOT}/scripts/package-proxy-merge.py"
  local tmp
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write the gradle init script ${init_script}"
    log "would set up the gradle token in ${gradle_props}"
    return 0
  fi
  mkdir -p "$(dirname "${init_script}")"
  (
    umask 077
    tmp="$(mktemp "${init_script}.tmp.XXXXXX")"
    cat >"${tmp}" <<'KTS'
// Route dependency resolution through the packages proxy. First-party GitHub
// Packages and local (file) repositories stay as they are; every other Maven
// repository keeps its place but has its URL rewritten to the proxy's
// equivalent, with the proxy's credentials. Nothing is added or removed, so
// repository order, the settings-versus-project choice and
// FAIL_ON_PROJECT_REPOS all behave as the build declared them.
fun proxyUrlFor(url: java.net.URI): String? {
    val host = url.host ?: return null
    if (url.scheme == "file" || host == "maven.pkg.github.com" || host == "packages.jorisjonkers.dev") return null
    val path = url.path ?: ""
    return when {
        host == "repo.spring.io" && path.startsWith("/milestone") -> "https://packages.jorisjonkers.dev/maven/spring-milestone/"
        host.endsWith("sonatype.org") ||
            host == "central.sonatype.com" && path.contains("snapshots") -> "https://packages.jorisjonkers.dev/maven/sonatype-snapshots/"
        host == "maven.google.com" || host == "dl.google.com" -> "https://packages.jorisjonkers.dev/maven/google-maven/"
        else -> "https://packages.jorisjonkers.dev/maven/" // Maven Central, the Gradle Plugin Portal, and any other mirror of Central
    }
}

fun RepositoryHandler.usePackagesProxy() {
    all {
        val repo = this as? MavenArtifactRepository ?: return@all
        val target = runCatching { proxyUrlFor(repo.url) }.getOrNull() ?: return@all
        repo.setUrl(target)
        repo.packagesProxyAuthentication()
    }
}

// The token is never in this file (it is world-readable): Gradle takes it
// from the property `packagesProxyToken` -- the -PpackagesProxyToken of the
// command line, else the gradle.properties of the Gradle user home -- or
// from the environment, and a build without any of them runs keyless.
fun packagesProxyToken(): String {
    (gradle.startParameter.projectProperties["packagesProxyToken"] as? String)
        ?.takeIf { it.isNotEmpty() }?.let { return it }
    val userHomeProperties = java.io.File(gradle.gradleUserHomeDir, "gradle.properties")
    if (userHomeProperties.isFile) {
        val properties = java.util.Properties()
        userHomeProperties.inputStream().use { properties.load(it) }
        val fromUserHome = properties.getProperty("packagesProxyToken")
        if (!fromUserHome.isNullOrEmpty()) return fromUserHome
    }
    return System.getenv("PACKAGES_PROXY_TOKEN") ?: ""
}

fun MavenArtifactRepository.packagesProxyAuthentication() {
    credentials(HttpHeaderCredentials::class) {
        name = "Authorization"
        value = "Bearer " + packagesProxyToken()
    }
    authentication {
        create<HttpHeaderAuthentication>("header")
    }
}

settingsEvaluated {
    pluginManagement.repositories.usePackagesProxy()
    dependencyResolutionManagement.repositories.usePackagesProxy()
}

beforeProject {
    buildscript.repositories.usePackagesProxy()
    repositories.usePackagesProxy()
}
KTS
    mv -f "${tmp}" "${init_script}"
  )
  chmod 644 "${init_script}"
  ok "packages proxy: ${init_script} is set up"
  printf 'packagesProxyToken=%s\nsystemProp.gradle.wrapperUser=packages\nsystemProp.gradle.wrapperPassword=%s\n' "${key}" "${key}" \
    | python3 "${helper}" "${gradle_props}" packages-proxy --mode 600 \
      --refuse '^(packagesProxyToken=|systemProp\.gradle\.wrapper(User|Password)=)' >/dev/null
  case $? in
    0) ok "packages proxy: ${gradle_props} is set up" ;;
    3) warn "packages proxy: ${gradle_props} has its own proxy line, nothing written" ;;
    *) return 1 ;;
  esac
}

# pp_write_maven: Maven's settings.xml, create-only. A root with no
# .m2/settings.xml gets one -- written through a temp file under umask
# 077 and renamed, so the key is never on disk world-readable -- with
# the proxy mirror (everything external but github) and its server; a
# root with its own settings.xml is never edited: the snippet to add
# is logged with the password as the PACKAGES_PROXY_KEY placeholder,
# so the run's output never carries the key either.
pp_write_maven() {
  local root="$1" key="$2"
  local settings="${root}/.m2/settings.xml"
  local tmp
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set up the packages proxy mirror and server in ${settings}"
    return 0
  fi
  if [ -f "${settings}" ]; then
    log "packages proxy: ${settings} already exists, add this to it (PACKAGES_PROXY_KEY stands for the key):"
    cat <<'XML'
  <mirrors>
    <mirror>
      <id>packages-proxy</id>
      <url>https://packages.jorisjonkers.dev/maven/</url>
      <mirrorOf>external:*,!github</mirrorOf>
    </mirror>
  </mirrors>
  <servers>
    <server>
      <id>packages-proxy</id>
      <username>packages</username>
      <password>PACKAGES_PROXY_KEY</password>
    </server>
  </servers>
XML
    return 0
  fi
  mkdir -p "$(dirname "${settings}")"
  (
    umask 077
    tmp="$(mktemp "${settings}.tmp.XXXXXX")"
    cat >"${tmp}" <<XML
<?xml version="1.0" encoding="UTF-8"?>
<settings>
  <mirrors>
    <mirror>
      <id>packages-proxy</id>
      <url>https://packages.jorisjonkers.dev/maven/</url>
      <mirrorOf>external:*,!github</mirrorOf>
    </mirror>
  </mirrors>
  <servers>
    <server>
      <id>packages-proxy</id>
      <username>packages</username>
      <password>${key}</password>
    </server>
  </servers>
</settings>
XML
    mv -f "${tmp}" "${settings}"
  )
  chmod 600 "${settings}"
  ok "packages proxy: ${settings} is set up"
}

# package_proxy_setup: refuse without a home to write into, skip a cloud
# session, report only under --check; then resolve the key, verify it,
# store it and call the eight writers -- netrc, go env, npmrc, uv, pip,
# cargo, gradle and maven -- each guarded, so a writer that fails warns
# without stopping the run; a missing or refused key warns and skips --
# no other step depends on it. The verify call carries the key in a
# stdin ("-K -"), never on argv or in a log line.
package_proxy_setup() {
  local root="${1:-}" key
  if [ -z "${root}" ] || [ "${root}" = "/" ]; then
    warn "packages proxy: no home directory to configure for the packages proxy, skipped"
    return 1
  fi
  if [ "${CLOUD}" = 1 ]; then
    log "packages proxy: the cloud session brings its own"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would look up the packages proxy key and verify it against https://packages.jorisjonkers.dev/npm/typescript"
    log "would store it under jorisjonkers/packages-token and write .netrc, go env, .npmrc, uv, pip, cargo, gradle and maven"
    return 0
  fi
  if ! key="$(pp_resolve_key "${root}")"; then
    warn "packages proxy: no key found (PACKAGES_PROXY_TOKEN, vault or the prompt), skipped"
    return 0
  fi
  if ! printf 'header = "Authorization: Bearer %s"\n' "${key}" \
    | curl -sf -o /dev/null -K - https://packages.jorisjonkers.dev/npm/typescript; then
    warn "packages proxy: the key was refused by https://packages.jorisjonkers.dev/npm/typescript, nothing was written"
    return 0
  fi
  pp_store_key "${root}" "${key}" || return 1
  pp_write_netrc "${root}" "${key}" \
    || warn "packages proxy: the netrc writer failed, the run continues"
  pp_write_go "${root}" \
    || warn "packages proxy: the go env writer failed, the run continues"
  pp_write_npmrc "${root}" "${key}" \
    || warn "packages proxy: the npmrc writer failed, the run continues"
  pp_write_uv "${root}" \
    || warn "packages proxy: the uv writer failed, the run continues"
  pp_write_pip "${root}" \
    || warn "packages proxy: the pip writer failed, the run continues"
  pp_write_cargo "${root}" "${key}" \
    || warn "packages proxy: the cargo writer failed, the run continues"
  pp_write_gradle "${root}" "${key}" \
    || warn "packages proxy: the gradle writer failed, the run continues"
  pp_write_maven "${root}" "${key}" \
    || warn "packages proxy: the maven writer failed, the run continues"
  ok "packages proxy: the key is verified and stored"
}

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

# On NixOS, Nix installs rustup system-wide, but rustup keeps its default
# toolchain per user in ~/.rustup: a fresh machine has rustup and no
# toolchain, so cargo fails. A repository's own rust-toolchain file or a
# mise pin overrides the default on its own, so an existing default is
# kept as it is.
# --- rustup default toolchain
rustup_default_toolchain() {
  [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ] || return 0
  if ! command -v rustup >/dev/null 2>&1; then
    warn "rustup: not on PATH; on NixOS it comes from nixos-modules' devWorkstation"
    return 0
  fi
  local current
  if current=$(rustup default 2>/dev/null); then
    # First line only, in plain bash: the toolchain name comes first.
    ok "rustup: default toolchain ${current%%$'\n'*} kept"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would run: rustup default stable && rustup component add clippy rustfmt rust-src"
    return 0
  fi
  if run rustup default stable && run rustup component add clippy rustfmt rust-src; then
    ok "rustup: default toolchain stable with clippy, rustfmt and rust-src"
  else
    warn "rustup: setting the default toolchain failed; run rustup default stable"
  fi
  return 0
}

# Sets the given top-level keys in a JSON config under $HOME, keeping
# every other key. Used for plugins that read a user config file.
_merge_user_config() {
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set $2 in $1"
    return 0
  fi
  python3 - "$1" "$2" <<'PY'
import json, pathlib, sys
path, want = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])
data = json.loads(path.read_text() or '{}') if path.exists() else {}
data.update(want)
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(data, indent=2) + "\n")
back = json.loads(path.read_text())
if any(back.get(k) != v for k, v in want.items()):
    raise SystemExit(f"{path}: {want} did not read back")
PY
}

# Joins the private-bank deny rules to each Claude profile's own; nothing else changes.
claude_private_denies() {
  [ "${CLOUD}" = 1 ] && return 0
  local profile_dir
  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would add the private-bank deny rules to ${profile_dir}/settings.json"
      continue
    fi
    python3 - "${profile_dir}/settings.json" '["Read(~/.vibe/**)", "Edit(~/.vibe/**)", "Read(~/.vibe-private/**)", "Edit(~/.vibe-private/**)", "Bash(*.vibe*)", "Read(~/.config/agent-kit/private/**)", "Edit(~/.config/agent-kit/private/**)", "Bash(*agent-kit/private*)", "Bash(*agent-kit-private*)"]' <<'PY' \
      && ok "private-bank deny rules in ${profile_dir}/settings.json" \
      || fail "could not add the private-bank deny rules to ${profile_dir}/settings.json"
import json, pathlib, sys
path, rules = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])
data = json.loads(path.read_text() or '{}') if path.exists() else {}
deny = data.setdefault('permissions', {}).setdefault('deny', [])
missing = [r for r in rules if r not in deny]
if missing:
    deny.extend(missing)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + '\n')
back = json.loads(path.read_text())['permissions']['deny']
if any(r not in back for r in rules):
    raise SystemExit(f'{path}: deny rules did not read back')
PY
  done
}

# Joins the allow rules for MCP servers whose every tool runs without a
# prompt (registry: claude_allow_tools) to each Claude profile's own; nothing else changes.
claude_mcp_allows() {
  local profile_dir
  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would add the MCP allow rules to ${profile_dir}/settings.json"
      continue
    fi
    python3 - "${profile_dir}/settings.json" '["mcp__overleaf__*"]' <<'PY' \
      && ok "MCP allow rules in ${profile_dir}/settings.json" \
      || fail "could not add the MCP allow rules to ${profile_dir}/settings.json"
import json, pathlib, sys
path, rules = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])
data = json.loads(path.read_text() or '{}') if path.exists() else {}
allow = data.setdefault('permissions', {}).setdefault('allow', [])
missing = [r for r in rules if r not in allow]
if missing:
    allow.extend(missing)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + '\n')
back = json.loads(path.read_text()).get('permissions', {}).get('allow', [])
if any(r not in back for r in rules):
    raise SystemExit(f'{path}: allow rules did not read back')
PY
  done
}

# Hindsight Coding Agents, pinned and locked to recall only (registry: coding_agents).
hindsight_coding_agents() {
  if ! command -v npx >/dev/null 2>&1; then
    warn "npx is not on PATH; Hindsight Coding Agents not installed"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would install @vectorize-io/hindsight-coding-agents@0.8.1 for claude-code and codex"
    return 0
  fi
  if npx -y @vectorize-io/hindsight-coding-agents@0.8.1 install claude-code codex --server self-hosted --api-url https://hindsight-api.jorisjonkers.dev </dev/null >/dev/null 2>&1; then
    ok "Hindsight Coding Agents 0.8.1 wired into claude-code and codex"
  else
    warn "Hindsight Coding Agents install failed"
    return 0
  fi
  local config="${HOME}/.hindsight/coding-agent.json"
  _merge_user_config "${config}" '{"apiUrl": "https://hindsight-api.jorisjonkers.dev", "autoInject": "recall", "autoSeed": false, "autoUpdate": false, "bankId": "default", "codebaseSurvey": false, "dynamicBankId": false, "gitIngest": "none", "injectTimeoutMs": 10000, "manageBankConfig": false, "pageRefreshEveryTurns": 1000, "pageTriggerType": "manual", "recallOptions": {"max_tokens": 1200}, "retainSessions": false, "toolGuideExtra": "Conversations are not captured automatically on this machine, whatever the guide above says: nothing is saved unless you save it. Save to memory only what is worth keeping long-term: decisions and their reasons, conventions, facts about people and the user, durable project context that the code and git history do not already record, and anything the user asks you to remember. Not small talk, not passing task details or progress, never secrets. Update an existing memory rather than adding a near-duplicate."}' \
    && ok "Hindsight Coding Agents locked to recall only (${config})" \
    || warn "could not write ${config}"
  # The token comes from HINDSIGHT_API_TOKEN; a copied one never stays on disk.
  python3 - "${config}" <<'PY' || warn "could not check ${config} for a token"
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text() or '{}') if path.exists() else {}
if data.pop('apiToken', None) is not None:
    path.write_text(json.dumps(data, indent=2) + '\n')
PY
  # Its installer registers the memory tools in the default profile only.
  run claude_each_profile claude mcp remove --scope user hindsight >/dev/null 2>&1 || true
  run claude_each_profile claude mcp add --scope user hindsight --env HINDSIGHT_MCP_HARNESS=claude-code \
    -- node "${HOME}/.hindsight/coding-agents/dist/mcp-server.js" >/dev/null 2>&1 \
    || warn "could not register the Hindsight memory tools in every Claude profile"
}

# Whether a marketplace is declared from the given folder (not tracking GitHub).
_marketplace_from_folder() {
  claude plugin marketplace list --json 2>/dev/null | python3 -c '
import json, os, sys
name, path = sys.argv[1], os.path.realpath(sys.argv[2])
ok = any(m.get("name") == name and m.get("source") == "directory"
         and os.path.realpath(m.get("path", "")) == path for m in json.load(sys.stdin))
sys.exit(0 if ok else 1)
' "$1" "$2"
  # end _marketplace_from_folder
}

# Rebuilds ROOT/ALIAS as the checkout ROOT/NAME under another name: its own
# marketplace.json with the name replaced, and symlinks to everything else.
_marketplace_alias() {
  local root="$1" name="$2" alias="$3" entry base
  [ -n "${alias}" ] && [ "${alias}" != "${name}" ] || return 1
  rm -rf "${root:?}/${alias}"
  mkdir -p "${root}/${alias}/.claude-plugin" || return 1
  for entry in "${root}/${name}"/* "${root}/${name}"/.[!.]* "${root}/${name}"/.claude-plugin/*; do
    [ -e "${entry}" ] || continue
    base="${entry#"${root}/${name}"/}"
    case "${base}" in .git|.claude-plugin|.claude-plugin/marketplace.json) continue ;; esac
    ln -s "${root}/${name}/${base}" "${root}/${alias}/${base}" || return 1
  done
  python3 - "${root}/${name}/.claude-plugin/marketplace.json" \
    "${root}/${alias}/.claude-plugin/marketplace.json" "${alias}" <<'PY'
import json, sys
source, dest, alias = sys.argv[1:]
manifest = json.load(open(source))
manifest['name'] = alias
json.dump(manifest, open(dest, 'w'), indent=2)
PY
  # end _marketplace_alias
}

# Checks a marketplace out at its pinned commit and declares it from that folder.
# Claude Code tracks a GitHub marketplace's default branch; a folder holds still.
# A reserved name is declared as ALIAS, from a renamed copy of the checkout.
ensure_marketplace() {
  local name="$1" repo="$2" commit="$3" alias="${4:-}"
  local root="${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}"
  local dir="${root}/${name}" declared="${4:-$1}" declared_dir="${root}/${4:-$1}"
  local base="${AGENT_KIT_GIT_BASE:-https://github.com}" moved=0
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would pin marketplace ${name} at ${commit:0:12}"
    return 0
  fi
  if [ "$(git -C "${dir}" rev-parse HEAD 2>/dev/null)" != "${commit}" ]; then
    mkdir -p "${dir}"
    [ -d "${dir}/.git" ] || git -C "${dir}" init -q
    git -C "${dir}" remote remove origin 2>/dev/null || true
    git -C "${dir}" remote add origin "${base}/${repo}"
    if ! git -C "${dir}" fetch -q --depth 1 origin "${commit}" 2>/dev/null \
       || ! git -C "${dir}" -c advice.detachedHead=false checkout -q --force FETCH_HEAD; then
      # Fail closed: only a folder at an earlier pin may keep serving plugins.
      if _marketplace_from_folder "${declared}" "${declared_dir}"; then
        warn "marketplace ${name}: could not fetch ${repo} at ${commit:0:12}; still at its previous pin"
      else
        unpinned_markets+=("${name}")
        warn "marketplace ${name}: could not fetch ${repo} at ${commit:0:12}; its plugins are not installed"
      fi
      return 0
    fi
    moved=1
  fi
  if [ "$(git -C "${dir}" rev-parse HEAD)" != "${commit}" ]; then
    fail "marketplace ${name}: the checkout is not at ${commit:0:12}"
    unpinned_markets+=("${name}")
    return 0
  fi
  if [ -n "${alias}" ] && ! _marketplace_alias "${root}" "${name}" "${alias}"; then
    unpinned_markets+=("${name}")
    warn "marketplace ${name}: could not build ${declared_dir}; its plugins are not installed"
    return 0
  fi
  if _marketplace_from_folder "${declared}" "${declared_dir}"; then
    if [ "${moved}" = 1 ]; then
      run claude plugin marketplace update "${declared}" >/dev/null 2>&1 \
        || warn "marketplace ${name}: could not refresh after moving to ${commit:0:12}"
    fi
    ok "marketplace ${name} pinned at ${commit:0:12}"
  else
    # Removing the upstream-tracking declaration uninstalls its plugins; the
    # plugins step reinstalls them from the pinned folder. A reserved name is
    # left alone: it cannot be declared from a folder, so its plugins would be lost.
    [ -n "${alias}" ] || run claude plugin marketplace remove "${name}" >/dev/null 2>&1 || true
    if run claude plugin marketplace add "${declared_dir}" >/dev/null 2>&1; then
      ok "marketplace ${name} pinned at ${commit:0:12} (${alias:+declared as ${alias}, }was tracking upstream)"
    else
      unpinned_markets+=("${name}")
      warn "marketplace ${name}: could not declare it from ${declared_dir}; its plugins are not installed"
    fi
  fi
  # end ensure_marketplace
}

# The commit a plugin installs at under a pinned marketplace: the marketplace
# commit for a plugin in its repo, or the sha its entry pins for an external one.
expected_plugin_commit() {
  python3 - "$1" "$2" "$3" <<'PY'
import json, pathlib, sys
folder, name, commit = sys.argv[1:]
try:
    manifest = json.loads((pathlib.Path(folder) / '.claude-plugin' / 'marketplace.json').read_text())
except (OSError, ValueError):
    raise SystemExit(0)
for plugin in manifest.get('plugins', []):
    if plugin.get('name') == name:
        source = plugin.get('source')
        print(source.get('sha', '') if isinstance(source, dict) else commit)
        break
PY
  # end expected_plugin_commit
}

# Removes a retired kit command, but only one setup wrote.
retire_kit_command() {
  local dest="${HOME}/.local/bin/$1"
  grep -qxF "# managed by agent-kit setup-workstation.sh" "${dest}" 2>/dev/null || return 0
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired ${dest}"
    return 0
  fi
  rm -f "${dest}" && ok "$1: retired, removed ${dest}"
}

# Retired: the kubectl port-forward that reached the in-cluster kubernetes MCP
# server (estate#316). The server now runs locally over stdio. Removes the
# launchd agent (macOS) and the systemd user unit (Linux) older setups left.
retire_port_forward_agents() {
  local prefix="dev.jorisjonkers.agent-kit.port-forward" file label
  local agents_dir="${AK_LAUNCH_AGENTS_DIR:-${HOME}/Library/LaunchAgents}"
  local units_dir="${AK_SYSTEMD_USER_DIR:-${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user}"
  local log_dir="${AK_LOG_DIR:-${HOME}/Library/Logs/agent-kit}"
  local found=0
  for file in "${agents_dir}/${prefix}".*.plist; do
    [ -e "${file}" ] || continue
    found=1
    label="$(basename "${file}" .plist)"
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would remove retired launchd agent ${label}"
      continue
    fi
    launchctl bootout "gui/$(id -u)/${label}" >/dev/null 2>&1 || true
    rm -f "${file}" && ok "${label}: retired, removed ${file}"
  done
  for file in "${units_dir}/${prefix}".*.service; do
    [ -e "${file}" ] || continue
    found=1
    label="$(basename "${file}")"
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would remove retired systemd unit ${label}"
      continue
    fi
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user disable --now "${label}" >/dev/null 2>&1 || true
    fi
    rm -f "${file}" && ok "${label}: retired, removed ${file}"
  done
  if [ "${CHECK_ONLY}" != 1 ]; then
    if [ "${found}" = 1 ] && command -v systemctl >/dev/null 2>&1; then
      systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
    rm -f "${log_dir}"/port-forward-*.log
  fi
}

# Copies a kit command onto PATH. A file there that setup did not write is left alone.
install_kit_command() {
  local name="$1"
  local src="${KIT_ROOT}/installer/${name}" dest="${HOME}/.local/bin/${name}"
  local marker="# managed by agent-kit setup-workstation.sh"
  if [ -e "${dest}" ] && ! grep -qxF "${marker}" "${dest}"; then
    warn "${dest} exists and was not written by setup; left alone"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would install ${dest}"
    return 0
  fi
  mkdir -p "$(dirname "${dest}")"
  install -m 0755 "${src}" "${dest}" && cmp -s "${src}" "${dest}" \
    && ok "${name} -> ${dest}" \
    || fail "${name}: could not install ${dest}"
}

# Points an agent home at the estate conventions, unless the user wrote their own.
# $1 is the AGENTS.md to write; $2 is shared (Shared Memory only) or private.
_agents_pointer() {
  local file="$1" kind="$2"
  local marker="<!-- managed by agent-kit setup-workstation.sh -->"
  if [ -f "${file}" ] && ! grep -qF "${marker}" "${file}"; then
    log "${file} is the user's own; left alone"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${file}"
    return 0
  fi
  mkdir -p "$(dirname "${file}")"
  if [ "${kind}" = private ]; then
    cat > "${file}" <<EOF
${marker}
# Agent contract

The estate-wide conventions live in one place:
https://github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md

Read it before doing anything non-trivial in an estate repository.

Multi-ticket builds (/implement-spec, a council run, a /flow feature in implementation): keep the build tracker current with the build-tracker skill.

## Memory

Two Hindsight banks are connected. At the start of a task, recall from both:
memory-private for personal context (people, plans, private notes) and
memory-api for estate and programming knowledge.

Save to memory-private only what is worth keeping long-term: facts about
people and the user, plans, decisions and their reasons, and anything the
user asks you to remember. Not small talk, not passing task details, never
secrets. Update an existing memory rather than adding a near-duplicate.
Never retain personal notes into memory-api: Hermes and every other agent
read it.
EOF
  else
    cat > "${file}" <<EOF
${marker}
# Agent contract

The estate-wide conventions live in one place:
https://github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md

Read it before doing anything non-trivial in an estate repository.

Multi-ticket builds (/implement-spec, a council run, a /flow feature in implementation): keep the build tracker current with the build-tracker skill.

## Memory

One Hindsight bank is connected: memory-api, for estate and programming
knowledge. At the start of a task, recall from it. Every agent reads it,
so never retain personal notes into it.
EOF
  fi
  ok "AGENTS.md pointer -> ${file}"
}

# Points a Vibe home at the estate conventions.
# $2 is shared (vibe, vibe-study: Shared Memory only) or private (vibe-private).
_vibe_agents_pointer() {
  _agents_pointer "${1}/AGENTS.md" "$2"
}

# The model gateway's pointer: one marked block in a client's global instruction
# file. Setup owns only that block; the text around it stays the user's.
gateway_pointer() {
  local file="$1" action="${2:-write}"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would ${action} the model-gateway pointer in ${file}"
    return 0
  fi
  if [ "${action}" = remove ]; then
    [ -f "${file}" ] || return 0
    python3 "${KIT_ROOT}/scripts/instruction-block.py" "${file}" model-gateway --remove >/dev/null \
      && ok "model-gateway pointer removed from ${file}" \
      || warn "model-gateway pointer: ${file} left alone; fix its block, then re-run setup"
    return 0
  fi
  mkdir -p "$(dirname "${file}")"
  python3 "${KIT_ROOT}/scripts/instruction-block.py" "${file}" model-gateway <<'BLOCK' \
    && ok "model-gateway pointer -> ${file}" \
    || warn "model-gateway pointer: ${file} left alone; fix its block, then re-run setup"
The model gateway is the `model-gateway` command on PATH. `model-gateway models`
lists the client models on this machine, each with its cost class and difficulty tier.
The `delegate` skill, which is coming later, plans delegations with it.
BLOCK
}

# The projects layout's rule: one marked block in a client's global instruction
# file. Setup owns only that block; the text around it stays the user's.
layout_pointer() {
  local file="$1" action="${2:-write}"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would ${action} the projects-layout rule in ${file}"
    return 0
  fi
  if [ "${action}" = remove ]; then
    [ -f "${file}" ] || return 0
    python3 "${KIT_ROOT}/scripts/instruction-block.py" "${file}" projects-layout --remove >/dev/null \
      && ok "projects-layout rule removed from ${file}" \
      || warn "projects-layout rule: ${file} left alone; fix its block, then re-run setup"
    return 0
  fi
  mkdir -p "$(dirname "${file}")"
  python3 "${KIT_ROOT}/scripts/instruction-block.py" "${file}" projects-layout <<'BLOCK' \
    && ok "projects-layout rule -> ${file}" \
    || warn "projects-layout rule: ${file} left alone; fix its block, then re-run setup"
Clones live at `~/Projects/<owner>/<repo>` (GitHub owner, exact case). Use an existing
clone instead of cloning again, even into /tmp. Worktrees live at
`~/Projects/<owner>/worktrees/<repo>/<name>`, never inside the repository:
`git -C ~/Projects/<owner>/<repo> worktree add ~/Projects/<owner>/worktrees/<repo>/<name> -b <branch>`.
In Claude Code, create the worktree that way, then call `EnterWorktree` with `path`, never `name`.
The projects-layout skill has the details.
BLOCK
}

# Every chat model the Mistral login can use, kept current in one Vibe home.
_vibe_models() {
  local home="$1"
  if VIBE_HOME="${home}" uv run --no-project --exclude-newer "7 days" --with tomlkit --with keyring python "${KIT_ROOT}/scripts/vibe-sync-models.py" \
      --config "${home}/config.toml" --has-key; then
    VIBE_HOME="${home}" uv run --no-project --exclude-newer "7 days" --with tomlkit --with keyring python "${KIT_ROOT}/scripts/vibe-sync-models.py" \
      --config "${home}/config.toml" \
      || warn "Vibe model sync failed (${home})"
  else
    warn "vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup"
  fi
}

# The kit's hooks into one Vibe home; extra arguments are --remove NAME pairs.
_vibe_hooks() {
  local home="$1"; shift
  uv run --no-project --exclude-newer "7 days" --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \
    "${KIT_ROOT}/installer/vibe-hooks.toml" --key hooks --config "${home}/hooks.toml" "$@" \
    && ok "Vibe hooks merged (${home})" \
    || fail "Vibe hooks merge failed (${home})"
}

# Once: the single Vibe home of before held sessions that recalled Private
# Memory, so it becomes vibe-private's and the shared home starts fresh.
# Until it has run, the shared home's Private Memory entry is what marks it as
# one to split: a failed split keeps setup out of both Vibe homes this run.
_vibe_split_homes() {
  local args=() out rc=0 line
  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)
  out=$(python3 "${KIT_ROOT}/scripts/vibe-split-homes.py" --shared "${VIBE_HOME}" --private "${VIBE_PRIVATE_HOME}" \
    ${args[@]+"${args[@]}"} 2>&1) || rc=$?
  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"
  if [ "${rc}" != 0 ]; then
    VIBE_HOMES_READY=0
    fail "could not split the Vibe home; both Vibe homes are left alone until it can"
  fi
}

# vibe-private's own Vibe home: the shared fleet plus Private Memory. Its
# sessions never share a directory with the password-free launchers.
_vibe_private_home() {
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would merge vibe-private MCP servers into ${VIBE_PRIVATE_HOME}/config.toml"
  else
    uv run --no-project --exclude-newer "7 days" --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \
      "${KIT_ROOT}/registry/generated/vibe-private/mcp-servers.toml" --config "${VIBE_PRIVATE_HOME}/config.toml" \
      && ok "vibe-private MCP servers merged" \
      || fail "vibe-private MCP merge failed"
    _vibe_models "${VIBE_PRIVATE_HOME}"
    _vibe_hooks "${VIBE_PRIVATE_HOME}" --remove agent-kit-retain-private
  fi
  _vibe_agents_pointer "${VIBE_PRIVATE_HOME}" private
  gateway_pointer "${VIBE_PRIVATE_HOME}/AGENTS.md"
  attribution_rule "${VIBE_PRIVATE_HOME}/AGENTS.md"
  vibe_no_commit_signature "${VIBE_PRIVATE_HOME}"
}

# The read-only critic agents the critique runner launches as
# `vibe --agent critic-<model>`. Called for the shared Vibe home only:
# vibe-private never runs critics. Nothing here touches config.toml, so
# no critic becomes the default agent.
_vibe_critic_agents() {
  local home="$1" src dest
  src="${KIT_ROOT}/registry/generated/vibe/agents/critic-mistral-large.toml"
  dest="${home}/agents/critic-mistral-large.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe critic agent critic-mistral-large -> ${dest}"
  else
    fail "Vibe critic agent critic-mistral-large: ${dest} does not match ${src}"
  fi
  src="${KIT_ROOT}/registry/generated/vibe/agents/critic-mistral-small.toml"
  dest="${home}/agents/critic-mistral-small.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe critic agent critic-mistral-small -> ${dest}"
  else
    fail "Vibe critic agent critic-mistral-small: ${dest} does not match ${src}"
  fi
  src="${KIT_ROOT}/registry/generated/vibe/agents/critic-glm.toml"
  dest="${home}/agents/critic-glm.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe critic agent critic-glm -> ${dest}"
  else
    fail "Vibe critic agent critic-glm: ${dest} does not match ${src}"
  fi
  src="${KIT_ROOT}/registry/generated/vibe/agents/critic-deepseek-flash.toml"
  dest="${home}/agents/critic-deepseek-flash.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe critic agent critic-deepseek-flash -> ${dest}"
  else
    fail "Vibe critic agent critic-deepseek-flash: ${dest} does not match ${src}"
  fi
  return 0
}

# The delegate agents the model gateway launches as
# `vibe --agent delegate-<what it may do>`. Called for the shared Vibe
# home only: vibe-private never delegates. Nothing here touches
# config.toml, so no delegate becomes the default agent.
_vibe_delegate_agents() {
  local home="$1" src dest
  src="${KIT_ROOT}/registry/generated/vibe/agents/delegate-readonly.toml"
  dest="${home}/agents/delegate-readonly.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe delegate agent delegate-readonly -> ${dest}"
  else
    fail "Vibe delegate agent delegate-readonly: ${dest} does not match ${src}"
  fi
  src="${KIT_ROOT}/registry/generated/vibe/agents/delegate-write.toml"
  dest="${home}/agents/delegate-write.toml"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${dest}"
  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then
    ok "Vibe delegate agent delegate-write -> ${dest}"
  else
    fail "Vibe delegate agent delegate-write: ${dest} does not match ${src}"
  fi
  return 0
}

# A shell -c wrapper for the pipe-into-sh installers the upstreams publish.
run_sh() {
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would run: $1"
    return 0
  fi
  bash -c "$1"
}

# Cloud mode registers a ${VAR} that Claude Code expands per session, keeping tokens out of the snapshot.
secret_ref() {
  if [ "${CLOUD}" = 1 ]; then
    # shellcheck disable=SC2016
    printf '${%s%s}' "$1" "${2:+:-}"
  else
    printf '%s' "${!1:-}"
  fi
}

# Runs argv once per Claude profile, each with its own config root.
# MCP servers live in a profile's own .claude.json, so the fleet has to
# be registered per profile; plugins and skills do not, because those
# directories are shared by symlink.
claude_each_profile() {
  local dir rc=0
  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
    claude_in_profile "${dir}" "$@" || rc=1
  done
  return "${rc}"
}

# An `auth: oauth` server is registered by URL only: each client signs in
# itself and keeps its own token, so setup stores none. This reports every
# client that has not signed in yet, under --check too.
oauth_status() {
  local name="$1" dir
  if command -v claude >/dev/null 2>&1; then
    for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
      claude_in_profile "${dir}" claude mcp get "${name}" 2>/dev/null | grep -q Connected \
        || warn "${name}: needs sign-in (claude ${dir}): run /mcp in Claude and pick ${name}"
    done
  fi
  if command -v codex >/dev/null 2>&1; then
    codex mcp list --json 2>/dev/null | uv run --no-project python -c \
      'import json,sys; sys.exit(0 if any(s.get("name") == sys.argv[1] and s.get("auth_status") not in (None, "not_logged_in") for s in json.load(sys.stdin)) else 1)' \
      "${name}" || warn "${name}: needs sign-in (codex): run codex mcp login ${name}"
  fi
  # Vibe keeps an MCP OAuth token in the keychain as mcp-oauth:<name>:tokens.
  # Only the macOS Keychain can be asked without reading the token; on Linux
  # the check is skipped, and that is reported.
  if command -v vibe >/dev/null 2>&1; then
    if [ "$(uname -s)" = Darwin ] && command -v security >/dev/null 2>&1; then
      security find-generic-password -s ai.mistral.vibe -a "mcp-oauth:${name}:tokens" >/dev/null 2>&1 \
        || warn "${name}: needs sign-in (vibe): run /mcp login ${name} in Vibe"
    else
      warn "${name}: vibe sign-in not checked: the check reads the macOS Keychain; skipped on $(uname -s)"
    fi
  fi
}

# Antigravity signs in through a browser once, so an unsigned machine only
# fails later, in a non-interactive run. `agy models` lists the account's
# models without calling one, so it spends no quota, and asks for a sign-in
# when there is none. Stdin is closed and a perl alarm ends it after 20s;
# signed in it answers in about a second. Only the CLI can be asked: the
# desktop app on its own is not checked. Reported under --check too.
report_antigravity_signin() {
  command -v agy >/dev/null 2>&1 || return 0
  local out rc=0 lower line=""
  if command -v perl >/dev/null 2>&1; then
    out="$(perl -e 'alarm shift; exec @ARGV or exit 127' 20 agy models </dev/null 2>&1)" || rc=$?
  else
    out="$(agy models </dev/null 2>&1)" || rc=$?
  fi
  if [ "${rc}" = 0 ]; then
    return 0
  fi
  lower="$(printf '%s' "${out}" | tr '[:upper:]' '[:lower:]')"
  case "${lower}" in
    *"sign in"*)
      warn "antigravity: needs sign-in: run agy and sign in with your Google account"
      return 0 ;;
  esac
  # 142 is SIGALRM: the alarm ended a call that did not answer.
  if [ "${rc}" = 142 ]; then
    warn "antigravity: agy models failed: no answer within 20s"
    return 0
  fi
  while IFS= read -r line; do
    case "${line}" in
      *[![:space:]]*) [ "${line}" = 'Fetching available models...' ] || break ;;
    esac
    line=""
  done <<< "${out}"
  warn "antigravity: agy models failed: ${line:-exit ${rc}}"
}

# Removes retired and undeclared MCP servers from every surface; the keep file spares undeclared ones.
converge_mcp() {
  local out rc=0 line dir
  local args=(--keep "${HOME}/.config/agent-kit/keep.yaml" --claude-home "${CLAUDE_HOME}")
  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do args+=(--claude "${dir}"); done
  command -v codex >/dev/null 2>&1 && args+=(--codex "${CODEX_HOME}/config.toml")
  # After a failed split the shared home's Private Memory entry must stay: it
  # is what marks the home for the next run's split.
  [ "${VIBE_HOMES_READY}" = 1 ] \
    && args+=(--vibe "${VIBE_HOME}/config.toml" --vibe-private "${VIBE_PRIVATE_HOME}/config.toml")
  [ "${OPENCODE_READY}" = 1 ] && [ "${OPENCODE_V1}" != 1 ] && command -v opencode >/dev/null 2>&1 \
    && args+=(--opencode "${OPENCODE_HOME}/opencode.json")
  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)
  if ! command -v uv >/dev/null 2>&1; then
    warn "uv is not on PATH; MCP servers not converged"
    return 0
  fi
  out=$(uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python "${KIT_ROOT}/scripts/converge.py" mcp \
    "${args[@]}" \
    --declared claude=memory-api,n8n,n8n-mcp,kubernetes,grimoire,outline,playwright,drawio,overleaf,hindsight \
    --declared codex=memory-api,n8n,n8n-mcp,kubernetes,grimoire,outline,playwright,drawio,overleaf,hindsight \
    --declared vibe=memory-api,n8n,n8n-mcp,kubernetes,grimoire,outline,playwright,drawio,overleaf,hindsight \
    --declared vibe-private=memory-api,memory-private,n8n,n8n-mcp,kubernetes,grimoire,outline,playwright,drawio,overleaf,hindsight \
    --declared opencode=memory-api,n8n,n8n-mcp,kubernetes,grimoire,outline,playwright,drawio,overleaf \
    --retired memory-mcp,vuetify --private memory-private 2>&1) || rc=$?
  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"
  [ "${rc}" = 0 ] || fail "MCP servers did not converge (see above)"
}

# Removes retired and undeclared marketplaces and plugins; the keep file spares undeclared ones.
converge_plugins() {
  local out rc=0 line
  local args=(--keep "${HOME}/.config/agent-kit/keep.yaml")
  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)
  if ! command -v uv >/dev/null 2>&1; then
    warn "uv is not on PATH; plugins not converged"
    return 0
  fi
  out=$(uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python "${KIT_ROOT}/scripts/converge.py" plugins \
    "${args[@]}" --declared-markets agent-kit-official,caveman,drawio,kubernetes-skill,ai-software-architect \
    --declared-plugins caveman@caveman,drawio@drawio,frontend-design@agent-kit-official,mattpocock-skills@agent-kit-official,security-guidance@agent-kit-official,kubernetes-skill@kubernetes-skill,github@agent-kit-official,ai-software-architect@ai-software-architect,typescript-lsp@agent-kit-official,pyright-lsp@agent-kit-official,kotlin-lsp@agent-kit-official,jdtls-lsp@agent-kit-official,ruby-lsp@agent-kit-official,gopls-lsp@agent-kit-official,rust-analyzer-lsp@agent-kit-official,lua-lsp@agent-kit-official,clangd-lsp@agent-kit-official,php-lsp@agent-kit-official,csharp-lsp@agent-kit-official,swift-lsp@agent-kit-official,liquid-lsp@agent-kit-official \
    --retired-markets hindsight \
    --retired-plugins hindsight-memory@hindsight 2>&1) || rc=$?
  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"
  [ "${rc}" = 0 ] || fail "plugins did not converge (see above)"
}

# -----------------------------------------------------------------
# The Gold Band agents, validated before anything is written.
# converge.py flattens gold_band.agents into the gold-band-agents
# crate's line protocol and the crate refuses a declaration that breaks
# an owner rule (#332): no Codex anywhere, no OpenRouter, OpenCode
# models only from opencode-go/, no literal secret, a well-formed id
# declared once, only the fields the crate knows. Its verdict is logged,
# and a refusal stops the run here; nothing was written yet, so nothing
# needs unwinding. A failure that is no verdict -- uv itself, or
# converge.py's own (exit 3) -- stops the run with its own message, so
# nothing is written either way. Without a built crate converge.py says
# so and the declarations are not validated.
# -----------------------------------------------------------------
gold_band_validate() {
  local out rc=0 line refusal=0
  if ! command -v uv >/dev/null 2>&1; then
    warn "uv is not on PATH; the Gold Band declarations are not validated"
    return 0
  fi
  out=$(uv run --no-project --with pyyaml --with tomlkit python "${KIT_ROOT}/scripts/converge.py" gold-band-validate \
    --registry "${KIT_ROOT}/registry/estate-tooling.yaml" 2>&1) || rc=$?
  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"
  [ "${rc}" = 0 ] && return 0
  # Exit 1 with the canary crate's line in the output is the
  # gold-band-agents crate's refusal: converge.py prints that line
  # whenever it ran at all, so an exit 1 without it is uv itself
  # failing to run converge.py, and converge.py's own failures exit 3.
  # Either way nothing arrived that the run could act on, so it stops
  # with nothing written: a refusal is refused, a broken step fails
  # closed.
  if [ "${rc}" = 1 ]; then
    case "${out}" in
      *"agent-kit-setup: "*) refusal=1 ;;
    esac
  fi
  if [ "${refusal}" = 1 ]; then
    fail "the Gold Band registry is refused; nothing was written"
  else
    fail "the Gold Band declarations could not be validated (exit ${rc}; see above)"
  fi
  exit 1
}

# An explicit CLAUDE_CONFIG_DIR=~/.claude moves .claude.json inside it, where a bare `claude` never looks.
claude_in_profile() {
  local dir="$1"; shift
  if [ "${dir}" = "${CLAUDE_HOME}" ] && [ -z "${CLAUDE_CONFIG_DIR:-}" ]; then
    "$@"
  else
    CLAUDE_CONFIG_DIR="${dir}" "$@"
  fi
}

# Links one shared surface of the primary profile into a secondary one.
# A real file or directory already sitting at the destination is left
# alone: that is someone's own config, and a symlink cannot give back
# what replacing it would lose.
link_profile_path() {
  local primary="$1" secondary="$2" rel="$3"
  local src="${primary}/${rel}" dest="${secondary}/${rel}"
  # Linking a root onto itself replaces every shared surface with a
  # symlink to itself, and the profile stops loading anything.
  if [ "${primary}" = "${secondary}" ] || [ "${primary}" -ef "${secondary}" ]; then
    fail "profile: ${secondary} is the primary root; refusing to link it onto itself"
    return 0
  fi
  # A shared DIRECTORY that the primary does not have yet is created, so
  # the link exists before the thing it points at does and whatever
  # writes there later reaches both profiles. A shared FILE is not
  # invented: an empty settings.json would look like a real answer.
  if [ ! -e "${src}" ]; then
    case "${rel}" in
      *.*)
        warn "profile: ${src} does not exist yet; ${rel} not shared"
        unshared_profile_paths+=("${dest}: ${src} does not exist")
        return 0
        ;;
      *)
        if [ "${CHECK_ONLY}" = 1 ]; then
          log "would create ${src}"
        else
          mkdir -p "${src}"
        fi
        ;;
    esac
  fi
  if [ -L "${dest}" ] && [ "$(readlink "${dest}")" = "${src}" ]; then
    ok "profile: ${dest} -> ${src}"
    return 0
  fi
  if [ -e "${dest}" ] && [ ! -L "${dest}" ]; then
    warn "profile: ${dest} exists and is not a symlink; left alone"
    unshared_profile_paths+=("${dest}: real file or directory, not replaced")
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would link ${dest} -> ${src}"
    return 0
  fi
  mkdir -p "$(dirname "${dest}")"
  if ln -sfn "${src}" "${dest}"; then
    ok "profile: ${dest} -> ${src}"
  else
    fail "profile: could not link ${dest} -> ${src}"
  fi
}

# Writes the claude-<profile> launcher for one secondary profile, so the
# profile is a command rather than an environment variable to remember.
# A file at that path that this script did not write is left alone.
install_profile_launcher() {
  local name="$1" dir="$2"
  local bin_dir="${CLAUDE_LAUNCHER_DIR:-$HOME/.local/bin}"
  local launcher="${bin_dir}/claude-${name}"
  local marker="# managed by agent-kit setup-workstation.sh"
  local content
  content="$(printf '%s\n' '#!/usr/bin/env bash' "${marker}" \
    "# Claude Code with the ${name} profile config root." \
    "export CLAUDE_CONFIG_DIR=$(printf '%q' "${dir}")" \
    'exec claude "$@"')"
  if [ -e "${launcher}" ] && ! grep -qxF "${marker}" "${launcher}"; then
    warn "profile: ${launcher} exists and was not written by setup; left alone"
    unshared_profile_paths+=("${launcher}: not a managed launcher, not replaced")
    return 0
  fi
  if [ -x "${launcher}" ] && [ "$(cat "${launcher}")" = "${content}" ]; then
    ok "profile: ${launcher}"
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would write ${launcher}"
  elif mkdir -p "${bin_dir}" && printf '%s\n' "${content}" > "${launcher}" \
    && chmod 755 "${launcher}"; then
    ok "profile: wrote ${launcher}"
  else
    fail "profile: could not write ${launcher}"
    return 0
  fi
  case ":${PATH}:" in
    *":${bin_dir}:"*) ;;
    *) warn "profile: ${bin_dir} is not on PATH; claude-${name} will not be found" ;;
  esac
}

# This script lives in <kit>/installer, and the first-party skills it
# copies live in <kit>/skills.
KIT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# The work profile lives in the default Claude directory, whatever
# CLAUDE_CONFIG_DIR the calling shell carries (a personal session exports
# its own). Dropping it keeps every claude call below on that directory.
CLAUDE_HOME="${HOME}/.claude"
if [ -n "${CLAUDE_CONFIG_DIR:-}" ] && [ "${CLAUDE_CONFIG_DIR%/}" != "${CLAUDE_HOME}" ]; then
  log "ignoring CLAUDE_CONFIG_DIR=${CLAUDE_CONFIG_DIR}; the work profile installs into ${CLAUDE_HOME}"
fi
unset CLAUDE_CONFIG_DIR
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
VIBE_HOME="${VIBE_HOME:-$HOME/.vibe}"
# vibe-private exports its own home as VIBE_HOME; setup run from inside a
# vibe-private session must still treat ~/.vibe as the shared home.
[ "${VIBE_HOME%/}" = "${HOME}/.vibe-private" ] && VIBE_HOME="${HOME}/.vibe"
# vibe-private's own Vibe home; the vibe-private launcher uses the same path.
VIBE_PRIVATE_HOME="${HOME}/.vibe-private"
# Antigravity's own global config, shared by the app, the CLI and the IDE.
# Never ~/.gemini itself: Gemini CLI reads the AGENTS.md and GEMINI.md there.
ANTIGRAVITY_HOME="${HOME}/.gemini/config"
# The CLI reads skills from its own folder as well as the global one.
ANTIGRAVITY_CLI_HOME="${HOME}/.gemini/antigravity-cli"
# 0 where Antigravity is not a supported surface: setup then skips it whole,
# rather than leaving half of it behind in ~/.gemini.
ANTIGRAVITY_READY=0
[ "$(uname -s)" = Darwin ] && ANTIGRAVITY_READY=1
# OpenCode 2's global config: opencode.json, AGENTS.md and skills/.
OPENCODE_HOME="${XDG_CONFIG_HOME:-$HOME/.config}/opencode"
# 0 on a cloud run: OpenCode is a workstation surface only.
OPENCODE_READY=1
[ "${CLOUD}" = 1 ] && OPENCODE_READY=0
# 1 once the opencode on PATH reports a 1.x release: its MCP config is then left alone.
OPENCODE_V1=0
# 0 after a failed split of the Vibe home: nothing then touches either home.
VIBE_HOMES_READY=1

# The release this copy came from. Publishing a release stamps its tag here;
# a copy taken from a checkout keeps the placeholder.
AGENT_KIT_RELEASE_TAG='@AGENT_KIT_RELEASE_TAG@'

# Outside a checkout, re-run the copy in the release's kit bundle; --uninstall needs none of it.
use_kit_bundle() {
  [ -f "${KIT_ROOT}/registry/estate-tooling.yaml" ] && return 0
  if [ -n "${AGENT_KIT_BUNDLE_STAGE:-}" ]; then
    fail "the kit bundle at ${AGENT_KIT_BUNDLE_STAGE} is not a complete kit"
    exit 1
  fi
  local url="${AGENT_KIT_SKILLS_BUNDLE_URL:-}"
  # The bundle of this script's own release, never `latest`: installer and
  # bundle then always match, even while a newer release is publishing.
  if [ -z "${url}" ]; then
    case "${AGENT_KIT_RELEASE_TAG}" in
      v[0-9]*) url="https://github.com/JorisJonkers-dev/agent-kit/releases/download/${AGENT_KIT_RELEASE_TAG}/agent-kit-skills.tar.gz" ;;
      *)
        fail "this copy carries no release tag, so it has no kit bundle;" \
          "set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"
        exit 1
        ;;
    esac
  fi
  local stage expected actual
  stage="$(mktemp -d)"
  log "not an agent-kit checkout; fetching the kit bundle from ${url}"
  # The checksum first, then the tarball under a URL keyed by it: a release
  # asset ignores the query, but an override host behind a CDN may cache the
  # tarball longer than its .sha256 and serve the previous release.
  if ! curl -fsSL -o "${stage}/kit.tar.gz.sha256" "${url}.sha256"; then
    fail "kit bundle unavailable at ${url}; set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"
    exit 1
  fi
  expected="$(awk '{print $1}' "${stage}/kit.tar.gz.sha256")"
  if ! curl -fsSL -o "${stage}/kit.tar.gz" "${url}?sha256=${expected}"; then
    fail "kit bundle unavailable at ${url}; set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"
    exit 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${stage}/kit.tar.gz" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${stage}/kit.tar.gz" | awk '{print $1}')"
  fi
  if [ -z "${expected}" ] || [ "${expected}" != "${actual}" ]; then
    fail "kit bundle at ${url} failed its sha256 check" \
      "(expected ${expected:-<empty>}, got ${actual}); refusing to run it"
    exit 1
  fi
  if ! tar -xzf "${stage}/kit.tar.gz" -C "${stage}" \
     || [ ! -f "${stage}/installer/setup-workstation.sh" ]; then
    fail "kit bundle at ${url} does not carry installer/setup-workstation.sh"
    exit 1
  fi
  ok "kit bundle verified (sha256 ${actual})"
  # Under curl | bash the rest of this script is still on stdin: drain it so no
  # command below reads it as input and curl is not cut off mid-write.
  [ -t 0 ] || cat >/dev/null
  AGENT_KIT_BUNDLE_STAGE="${stage}" exec bash "${stage}/installer/setup-workstation.sh" \
    ${ORIG_ARGS[@]+"${ORIG_ARGS[@]}"} </dev/null
}
[ "${UNINSTALL}" = 1 ] || use_kit_bundle
if [ -n "${AGENT_KIT_BUNDLE_STAGE:-}" ]; then
  trap 'rm -rf "${AGENT_KIT_BUNDLE_STAGE}"' EXIT
fi

# Where install_linux commands install: system-wide when writable, as on a cloud VM.
if [ -w /usr/local/bin ]; then
  AK_BIN_DIR=/usr/local/bin AK_OPT_DIR=/usr/local/lib/agent-kit
else
  AK_BIN_DIR="$HOME/.local/bin" AK_OPT_DIR="$HOME/.local/lib/agent-kit"
fi
export AK_BIN_DIR AK_OPT_DIR

if [ "${CLOUD}" = 1 ]; then
  . "${KIT_ROOT}/installer/cloud-session.sh"
  cloud_prepare
fi

CLAUDE_HOOKS_DIR="${CLAUDE_HOME}/hooks"
CODEX_HOOKS_DIR="${CODEX_HOME}/hooks"

# -----------------------------------------------------------------
# The only agent hooks the estate ships are the critique hooks below
# (ADR-0002). This purges the three
# retired knowledge-recall hook groups (edit recall, git-commit
# capture, session digest) from Claude settings.json by COMMAND
# BASENAME, so an operator's own hooks survive, and removes the now-dead
# hook script files themselves. Runs on every setup AND on --uninstall,
# because a machine set up by the retired install.sh/install-agents.sh
# is exactly the one that still has these.
# -----------------------------------------------------------------
purge_retired_hooks() {
  # Claude's settings and Codex's hooks.json share the hook layout. Codex's file
  # now also holds Hindsight Coding Agents' hooks, so only the retired ones go.
  local settings
  for settings in "${CLAUDE_HOME}/settings.json" "${CODEX_HOME}/hooks.json"; do
  if [ ! -e "${settings}" ]; then
    ok "no retired knowledge hooks (${settings} does not exist)"
  elif ! command -v python3 >/dev/null 2>&1; then
    warn "python3 is not on PATH; cannot purge retired hooks from ${settings}"
  elif [ "${CHECK_ONLY}" = 1 ]; then
    if grep -qE 'pre-tool-use-edit-recall|pre-tool-use-git-commit-capture|stop-session-digest|kb-stop-digest|user-prompt-submit-recall|kb-user-prompt-recall' "${settings}" 2>/dev/null; then
      warn "retired knowledge hooks are still wired in ${settings}"
    else
      ok "no retired knowledge hooks in ${settings}"
    fi
  else
    if AK_SETTINGS_FILE="${settings}" python3 - <<'PURGE'
import json
import os
import pathlib
import sys

path = pathlib.Path(os.environ["AK_SETTINGS_FILE"])

RETIRED = {
    "pre-tool-use-edit-recall.sh",
    "pre-tool-use-git-commit-capture.sh",
    "stop-session-digest.sh",
    "kb-stop-digest.sh",
    "kb-user-prompt-recall.sh",
    "user-prompt-submit-recall.sh",
}

try:
    data = json.loads(path.read_text() or "{}")
except json.JSONDecodeError:
    print("purge: %s is not valid JSON; left untouched" % path, file=sys.stderr)
    raise SystemExit(0)
if not isinstance(data, dict):
    raise SystemExit(0)

hooks = data.get("hooks")
if not isinstance(hooks, dict):
    raise SystemExit(0)


def retired(group):
    for hook in group.get("hooks", []) if isinstance(group, dict) else []:
        command = hook.get("command", "") if isinstance(hook, dict) else ""
        for token in command.replace('"', " ").split():
            if token.rsplit("/", 1)[-1] in RETIRED:
                return True
    return False


removed = 0
for event in list(hooks):
    groups = hooks.get(event)
    if not isinstance(groups, list):
        continue
    kept = [g for g in groups if not retired(g)]
    removed += len(groups) - len(kept)
    if kept:
        hooks[event] = kept
    else:
        hooks.pop(event, None)

if hooks:
    data["hooks"] = hooks
else:
    data.pop("hooks", None)

path.write_text(json.dumps(data, indent=2) + "\n")

back = path.read_text()
still = sorted(name for name in RETIRED if name in back)
if still:
    print("purge: FAILED, still referenced: %s" % ", ".join(still), file=sys.stderr)
    raise SystemExit(1)
print("purge: removed %d retired hook group(s); none remain" % removed)
PURGE
    then
      ok "retired knowledge hooks purged from ${settings}"
    else
      fail "purge of retired knowledge hooks in ${settings} failed"
    fi
  fi
  done

  local stale
  for stale in \
    "${CLAUDE_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \
    "${CLAUDE_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \
    "${CLAUDE_HOOKS_DIR}/stop-session-digest.sh" \
    "${CLAUDE_HOOKS_DIR}/user-prompt-submit-recall.sh" \
    "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \
    "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \
    "${CODEX_HOOKS_DIR}/kb-stop-digest.sh" \
    "${CODEX_HOOKS_DIR}/kb-user-prompt-recall.sh"
  do
    if [ -e "${stale}" ]; then
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "would remove retired hook file ${stale}"
      else
        rm -f "${stale}"
        log "removed retired hook file ${stale}"
      fi
    fi
  done
  if [ "${CHECK_ONLY}" != 1 ]; then
    rmdir "${CLAUDE_HOOKS_DIR}" "${CODEX_HOOKS_DIR}" 2>/dev/null || true
  fi
}

# -----------------------------------------------------------------
# The owner's identity only: no commit or PR may credit an AI agent or a
# model provider. Each agent with a setting for its own attribution gets
# it turned off (Claude Code here, Mistral Vibe in each Vibe home), and
# each agent's global instruction file carries the rule. The git hook
# catches the rest, whichever agent or person wrote the message:
# installer/git-hooks/no-agent-attribution and its allowlist,
# installer/git-hooks/allowed-identities, wired as config hooks
# (hook.<name>.command), which run next to a repository's own hooks
# instead of replacing them as core.hooksPath would.
# git commit/push --no-verify still skips it.
# -----------------------------------------------------------------
claude_no_attribution() {
  local profile_dir
  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
    if _merge_user_config "${profile_dir}/settings.json" '{"attribution": {"commit": "", "pr": ""}, "includeCoAuthoredBy": false}'; then
      [ "${CHECK_ONLY}" = 1 ] || ok "no commit or PR attribution in ${profile_dir}/settings.json"
    else
      fail "could not turn off commit and PR attribution in ${profile_dir}/settings.json"
    fi
  done
}

# Mistral Vibe tells its model to sign every commit unless
# include_commit_signature is false in the home's config.toml.
vibe_no_commit_signature() {
  local home="$1"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would set include_commit_signature = false in ${home}/config.toml"
    return 0
  fi
  uv run --no-project --exclude-newer "7 days" --with tomlkit python "${KIT_ROOT}/scripts/vibe-no-commit-signature.py" \
    --config "${home}/config.toml" >/dev/null \
    && ok "no Vibe commit signature in ${home}/config.toml" \
    || fail "could not turn off the Vibe commit signature in ${home}/config.toml"
}

# The rule against agent attribution: one marked block in an agent's global
# instruction file. Setup owns only that block; the text around it stays the user's.
attribution_rule() {
  local file="$1"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would write the no-agent-attribution rule in ${file}"
    return 0
  fi
  mkdir -p "$(dirname "${file}")"
  python3 "${KIT_ROOT}/scripts/instruction-block.py" "${file}" no-agent-attribution <<'BLOCK' >/dev/null \
    && ok "no-agent-attribution rule -> ${file}" \
    || warn "no-agent-attribution rule: ${file} left alone; fix its block, then re-run setup"
Commits, pull requests and issues carry the owner's identity only. Never add a
Co-Authored-By trailer, a Generated by or Generated with line, a session link, or
any other credit to an AI agent, a model or a model provider, and never name one as
the author or reviewer of a change. A git hook refuses such commits.
Never describe AI use in a pull request, commit or issue either: not which agent
or model wrote, reviewed, coordinated or ran any part of the work, and not how
the work was delegated. Describe the change, why, and how it was checked.
BLOCK
}

# True when git, configured as above, refuses a commit that credits an
# agent and accepts one that does not. Run in a scratch repository.
_git_no_agent_attribution_works() {
  local repo rc=1
  repo="$(mktemp -d)" || return 1
  if git -C "${repo}" init -q \
    && ! git -C "${repo}" -c user.name=check -c user.email=check@example.invalid -c commit.gpgsign=false commit -q --allow-empty \
      -m "check" -m "Co-Authored-By: Mistral Vibe <vibe@mistral.ai>" >/dev/null 2>&1 \
    && git -C "${repo}" -c user.name=check -c user.email=check@example.invalid -c commit.gpgsign=false commit -q --allow-empty \
      -m "check" >/dev/null 2>&1; then
    rc=0
  fi
  rm -rf "${repo}"
  return "${rc}"
}

git_no_agent_attribution() {
  local dir="${HOME}/.config/agent-kit/git-hooks" event
  local dest="${dir}/no-agent-attribution"
  if ! command -v git >/dev/null 2>&1; then
    warn "git is not on PATH; the no-agent-attribution git hook is not installed"
    return 0
  fi
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would install the no-agent-attribution hook and its allowlist in ${dir} (commit-msg, pre-push)"
    return 0
  fi
  if ! mkdir -p "${dir}" \
    || ! install -m 0755 "${KIT_ROOT}/installer/git-hooks/no-agent-attribution" "${dest}" \
    || ! install -m 0644 "${KIT_ROOT}/installer/git-hooks/allowed-identities" "${dir}/allowed-identities"; then
    fail "could not install the no-agent-attribution git hook in ${dir}"
    return 0
  fi
  for event in commit-msg pre-push; do
    if ! git config --global "hook.agent-kit-no-attribution-${event}.command" "\"${dest}\" ${event}" \
      || ! git config --global "hook.agent-kit-no-attribution-${event}.event" "${event}"; then
      fail "could not register the no-agent-attribution ${event} hook in ~/.gitconfig"
      return 0
    fi
  done
  # A stubbed git (the test harness) prints no version; there is nothing to check.
  case "$(git --version 2>/dev/null)" in
    "git version "*) ;;
    *) ok "no-agent-attribution git hook registered for commit-msg and pre-push"; return 0 ;;
  esac
  if _git_no_agent_attribution_works; then
    ok "no-agent-attribution git hook refuses commits that credit an agent"
  else
    fail "the no-agent-attribution git hook is registered but did not refuse a test commit; $(git --version) may not run hooks from config (hook.<name>.command)"
  fi
}

# -----------------------------------------------------------------
# The runner entrypoint (runner/template/entrypoint.sh) once
# wrote its global git settings wherever it ran. On a workstation they
# name the credential helper agent-gh-app, which only the runner image
# ships, so every push printed "git: 'credential-agent-gh-app' is not a
# git command". This removes that helper and the two settings the same
# run wrote with it: credential.useHttpPath and safe.directory /workspace.
# Neither goes without the helper: alone, either may be the owner's own.
# Where git-credential-agent-gh-app is on PATH the settings are real.
# -----------------------------------------------------------------
purge_leaked_runner_git_config() {
  local helpers removed
  command -v git >/dev/null 2>&1 || return 0
  helpers="$(git config --global --get-all credential.helper 2>/dev/null || true)"
  printf '%s\n' "${helpers}" | grep -qxF agent-gh-app || return 0
  command -v git-credential-agent-gh-app >/dev/null 2>&1 && return 0
  if [ "${CHECK_ONLY}" = 1 ]; then
    warn "your ~/.gitconfig names the runner's credential helper agent-gh-app, which is not on PATH; run setup to remove it"
    return 0
  fi
  if ! git config --global --unset-all credential.helper '^agent-gh-app$'; then
    fail "could not remove the runner's credential helper agent-gh-app from ~/.gitconfig"
    return 0
  fi
  removed="credential.helper agent-gh-app"
  git config --global --unset-all credential.useHttpPath '^true$' 2>/dev/null \
    && removed="${removed}, credential.useHttpPath"
  git config --global --unset-all safe.directory '^/workspace$' 2>/dev/null \
    && removed="${removed}, safe.directory /workspace"
  ok "fixed: removed the runner's leaked git settings from ~/.gitconfig (${removed})"
}

# -----------------------------------------------------------------
# Run outside its container, the runner entrypoint once trusted its
# workspace root in the invoking user's .claude.json (a Claude project)
# and Codex config.toml (a [projects."<root>"] table). Where that root
# does not exist on this machine, the trust is that leak and nothing else,
# so it goes from every Claude profile setup manages and from Codex,
# together with the entries for the repositories under it.
# -----------------------------------------------------------------
purge_leaked_runner_workspace_trust() {
  local root="$1" dir out rc changed
  local files=()
  [ -e "${root}" ] && return 0
  command -v python3 >/dev/null 2>&1 || return 0
  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
    if [ "${dir}" = "${CLAUDE_HOME}" ] && [ -z "${CLAUDE_CONFIG_DIR:-}" ]; then
      files+=("${HOME}/.claude.json")
    else
      files+=("${dir}/.claude.json")
    fi
  done
  files+=("${CODEX_HOME}/config.toml")
  out="$(mktemp)"
  AK_RUNNER_WORKSPACE="${root}" AK_CHECK_ONLY="${CHECK_ONLY}" \
    python3 - "${files[@]}" >"${out}" <<'RUNNER_WORKSPACE'
import json
import os
import re
import sys
from pathlib import Path

ROOT = os.environ["AK_RUNNER_WORKSPACE"].rstrip("/")
CHECK_ONLY = os.environ["AK_CHECK_ONLY"] == "1"
TABLE = re.compile(r"^\s*\[\[?[^\[\]=,]+\]\]?\s*(?:#.*)?$")
PROJECT = re.compile(r"""^\s*\[\s*projects\s*\.\s*(?:"((?:[^"\\]|\\.)*)"|'([^']*)')\s*\]\s*(?:#.*)?$""")
LOOSE = re.compile(r"^\s*(?:#.*)?$")


def leaked(project):
    return project == ROOT or project.startswith(ROOT + "/")


def write(path, text):
    target = path.resolve()
    mode = target.stat().st_mode & 0o777
    staged = target.with_name(target.name + ".agent-kit-tmp")
    staged.write_text(text)
    os.chmod(str(staged), mode)
    os.replace(str(staged), str(target))


def claude(path):
    try:
        text = path.read_text()
        data = json.loads(text)
    except (OSError, ValueError):
        return False
    projects = data.get("projects") if isinstance(data, dict) else None
    if not isinstance(projects, dict):
        return False
    gone = [name for name in projects if leaked(name)]
    if gone and not CHECK_ONLY:
        for name in gone:
            del projects[name]
        write(path, json.dumps(data, indent=2, ensure_ascii=False) + ("\n" if text.endswith("\n") else ""))
    return bool(gone)


def codex(path):
    try:
        lines = path.read_text().splitlines(True)
    except OSError:
        return False
    kept, held, dropping, found = [], [], False, False
    for line in lines:
        if TABLE.match(line):
            if dropping:
                kept.extend(held)
            held = []
            match = PROJECT.match(line)
            name = None
            if match:
                name = match.group(1) if match.group(1) is not None else match.group(2)
            dropping = name is not None and leaked(name)
            if dropping:
                found = True
                while kept and not kept[-1].strip():
                    kept.pop()
                continue
        elif dropping:
            held = held + [line] if LOOSE.match(line) else []
            continue
        kept.append(line)
    if found and not CHECK_ONLY:
        write(path, "".join(kept))
    return found


for argument in dict.fromkeys(sys.argv[1:]):
    path = Path(argument)
    if path.is_file() and (codex(path) if path.suffix == ".toml" else claude(path)):
        print(argument)
RUNNER_WORKSPACE
  rc=$?
  changed="$(paste -s -d ' ' "${out}")"
  rm -f "${out}"
  if [ "${rc}" != 0 ]; then
    fail "could not check the Claude and Codex configs for the runner's ${root} trust"
    return 0
  fi
  [ -n "${changed}" ] || return 0
  if [ "${CHECK_ONLY}" = 1 ]; then
    warn "the runner's ${root} project trust leaked into ${changed}; run setup to remove it"
  else
    ok "fixed: removed the runner's leaked ${root} project trust from ${changed}"
  fi
}

# -----------------------------------------------------------------
# The critique hooks (docs/adr/0002-exactly-three-critique-hooks.md):
# merged by NAME into Claude Code's settings.json and Codex's
# hooks.json, so the operator's own hooks survive, and removed by
# --uninstall. Each command is a fixed path into the agent home's
# critique skill, never a version, so an upgrade leaves the definition
# Codex trusted unchanged. Codex still runs them only once the owner
# trusts them in /hooks; /flow setup checks that.
# -----------------------------------------------------------------
_critique_hooks() {
  local mode="$1" home="$2" file="$3" label="$4" spec="$5" names="$6"
  local path="${home}/${file}" out rc name
  if ! command -v python3 >/dev/null 2>&1; then
    warn "python3 is not on PATH; cannot merge the critique hooks into ${path}"
    return 0
  fi
  if [ "${mode}" = install ] && [ "${CHECK_ONLY}" != 1 ]; then
    for name in ${names}; do
      if [ ! -f "${home}/skills/${name}.sh" ]; then
        fail "critique hooks: ${home}/skills/${name}.sh is missing; not wired into ${path}"
        return 0
      fi
    done
  fi
  out="$(mktemp)"
  AK_HOOKS_FILE="${path}" AK_HOOKS_HOME="${home}" AK_HOOKS_MODE="${mode}" \
    AK_CHECK_ONLY="${CHECK_ONLY}" AK_HOOKS_SPEC="${spec}" AK_HOOKS_TIMEOUT=5 \
    python3 - >"${out}" <<'CRITIQUE_HOOKS'
import json
import os
import pathlib
import sys

path = pathlib.Path(os.environ["AK_HOOKS_FILE"])
home = os.environ["AK_HOOKS_HOME"]
mode = os.environ["AK_HOOKS_MODE"]
check_only = os.environ["AK_CHECK_ONLY"] == "1"
spec = json.loads(os.environ["AK_HOOKS_SPEC"])
timeout = int(os.environ["AK_HOOKS_TIMEOUT"])


def names(handler, name):
    command = handler.get("command", "") if isinstance(handler, dict) else ""
    if not isinstance(command, str):
        return False
    return any(token.rsplit("/", 1)[-1] == name + ".sh" for token in command.replace('"', " ").split())


def wanted(name, matcher, skill):
    script = "%s/skills/%s/hooks/%s.sh" % (home, skill, name)
    group = {"matcher": matcher} if matcher else {}
    group["hooks"] = [{"type": "command", "command": '/bin/sh "%s"' % script, "timeout": timeout}]
    return group


text = path.read_text() if path.exists() else None
if text is None and mode == "remove":
    print("unchanged")
    raise SystemExit(0)
try:
    data = json.loads(text or "{}")
except json.JSONDecodeError:
    print("%s is not valid JSON" % path, file=sys.stderr)
    raise SystemExit(2)
if not isinstance(data, dict) or not isinstance(data.get("hooks", {}), dict):
    print("%s has no hooks mapping setup can merge into" % path, file=sys.stderr)
    raise SystemExit(2)
before = json.loads(json.dumps(data))
hooks = data.setdefault("hooks", {})

for name, event, matcher, skill in spec:
    want = wanted(name, matcher, skill) if mode == "install" else None
    placed = False
    for current in list(hooks):
        groups = hooks[current]
        if not isinstance(groups, list):
            continue
        kept = []
        for group in groups:
            handlers = group.get("hooks") if isinstance(group, dict) else None
            if not isinstance(handlers, list) or not any(names(h, name) for h in handlers):
                kept.append(group)
                continue
            others = [h for h in handlers if not names(h, name)]
            if want is not None and not placed and current == event and not others:
                kept.append(want)
                placed = True
            elif others:
                kept.append(dict(group, hooks=others))
        if kept or not groups:
            hooks[current] = kept
        else:
            hooks.pop(current)
    if want is not None and not placed:
        hooks.setdefault(event, []).append(want)
if not hooks and before.get("hooks") != {}:
    data.pop("hooks", None)

if data == before:
    print("unchanged")
    raise SystemExit(0)
if check_only:
    print("would-change")
    raise SystemExit(0)
path.parent.mkdir(parents=True, exist_ok=True)
# Written in place, so a profile's settings.json symlink stays a symlink.
path.write_text(json.dumps(data, indent=2) + "\n")

back = json.loads(path.read_text()).get("hooks", {})
for name, event, matcher, skill in spec:
    found = [
        (current, group)
        for current, groups in back.items()
        if isinstance(groups, list)
        for group in groups
        if isinstance(group, dict) and any(names(h, name) for h in group.get("hooks", []))
    ]
    expected = [(event, wanted(name, matcher, skill))] if mode == "install" else []
    if found != expected:
        print("%s: %s is wired as %r, not %r" % (path, name, found, expected), file=sys.stderr)
        raise SystemExit(1)
print("changed")
CRITIQUE_HOOKS
  rc=$?
  name="$(cat "${out}")"
  rm -f "${out}"
  out="${name}"
  case "${rc}:${mode}:${out}" in
    0:install:changed) ok "critique hooks installed in ${label} (${path})" ;;
    0:remove:changed) ok "critique hooks removed from ${label} (${path})" ;;
    0:install:unchanged) ok "critique hooks already installed in ${label} (${path})" ;;
    0:remove:unchanged) ok "no critique hooks in ${label} (${path})" ;;
    0:install:would-change) log "would install the critique hooks in ${label} (${path})" ;;
    0:remove:would-change) log "would remove the critique hooks from ${label} (${path})" ;;
    2:*) warn "critique hooks: ${path} left untouched; fix it, then re-run setup" ;;
    *) fail "critique hooks: merging into ${path} failed" ;;
  esac
}

critique_hooks() {
  _critique_hooks "$1" "${CLAUDE_HOME}" settings.json 'Claude Code' '[["critique-commit-gate","PreToolUse","Bash","critique"],["critique-pr-gate","PreToolUse","Bash","critique"],["critique-session-start","SessionStart",null,"critique"],["projects-layout-guard","PreToolUse","Bash|EnterWorktree","projects-layout"]]' 'critique/hooks/critique-commit-gate critique/hooks/critique-pr-gate critique/hooks/critique-session-start projects-layout/hooks/projects-layout-guard'
  _critique_hooks "$1" "${CODEX_HOME}" hooks.json 'Codex' '[["critique-commit-gate","PreToolUse","Bash","critique"],["critique-pr-gate","PreToolUse","Bash","critique"],["critique-session-start","SessionStart",null,"critique"],["projects-layout-guard","PreToolUse","Bash","projects-layout"]]' 'critique/hooks/critique-commit-gate critique/hooks/critique-pr-gate critique/hooks/critique-session-start projects-layout/hooks/projects-layout-guard'
}

# -----------------------------------------------------------------
# --uninstall: remove every file the retired install.sh /
# install-agents.sh wrote (base skills, Spec Kit commands/skills, the
# knowledge-system version manifest), then purge retired hooks, remove
# the critique hooks from Claude Code and Codex, and exit. A normal run
# removes their MCP entries (converge_mcp). Beyond the critique hooks it
# does not touch anything this script itself manages -- a re-run with no
# flags reinstalls the current local_skills and the hooks afterward.
# -----------------------------------------------------------------
if [ "${UNINSTALL}" = 1 ]; then
  log "uninstalling files from a previous install.sh / install-agents.sh"
  legacy_remove() {
    if [ ! -e "$1" ]; then return 0; fi
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would remove $1"
    else
      rm -f "$1"
      log "removed $1"
    fi
  }
  legacy_remove "${CLAUDE_HOME}/commands/speckit.analyze.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.checklist.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.clarify.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.constitution.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.implement.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.plan.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.specify.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.tasks.md"
  legacy_remove "${CLAUDE_HOME}/commands/speckit.taskstoissues.md"
  legacy_remove "${CLAUDE_HOME}/skills/topics/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/audit/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/kb-first/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/token-economy/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/agent-session-bootstrap/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/grill-me/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/SKILL.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/council.mjs"
  legacy_remove "${CLAUDE_HOME}/skills/council/council.toml"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/_baseline.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/consolidator.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/correct_course.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/critic.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/design_consolidator.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/design_lens.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/design_vote.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/designer.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/grill.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/planner.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/review_triage.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewer_acceptance.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewer_adversarial.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewer_edgecase.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/checkpoint-1.html"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/checkpoint-1.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/checkpoint-2.html"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/checkpoint-2.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/design-checkpoint.html"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviewpack/design-checkpoint.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/reviser.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/story_author.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/story_check.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/story_template.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/survey.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/triage_judge.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/verifier.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/prompts/worker.md"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/amendment.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/consolidated.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/context-pack.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/design-ledger.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/events.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/plan.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/review-verdict.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/routing-verdict.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/run-state.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/verdict.schema.json"
  legacy_remove "${CLAUDE_HOME}/skills/council/schemas/worker-result.schema.json"
  legacy_remove "${CLAUDE_HOME}/.knowledge-system-version"
  legacy_remove "${CODEX_HOME}/skills/speckit-analyze/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-checklist/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-clarify/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-constitution/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-implement/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-plan/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-specify/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-tasks/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/speckit-taskstoissues/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/topics/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/audit/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/kb-first/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/token-economy/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/agent-session-bootstrap/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/grill-me/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/council/SKILL.md"
  legacy_remove "${CODEX_HOME}/skills/council/council.mjs"
  legacy_remove "${CODEX_HOME}/skills/council/council.toml"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/_baseline.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/consolidator.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/correct_course.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/critic.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/design_consolidator.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/design_lens.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/design_vote.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/designer.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/grill.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/planner.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/review_triage.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewer_acceptance.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewer_adversarial.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewer_edgecase.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/checkpoint-1.html"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/checkpoint-1.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/checkpoint-2.html"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/checkpoint-2.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/design-checkpoint.html"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviewpack/design-checkpoint.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/reviser.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/story_author.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/story_check.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/story_template.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/survey.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/triage_judge.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/verifier.md"
  legacy_remove "${CODEX_HOME}/skills/council/prompts/worker.md"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/amendment.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/consolidated.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/context-pack.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/design-ledger.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/events.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/plan.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/review-verdict.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/routing-verdict.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/run-state.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/verdict.schema.json"
  legacy_remove "${CODEX_HOME}/skills/council/schemas/worker-result.schema.json"
  legacy_remove "${CODEX_HOME}/.knowledge-system-version"
  purge_retired_hooks
  critique_hooks remove
  log "uninstall complete: ${failures} failure(s), ${warnings} warning(s)"
  [ "${failures}" = 0 ]
  exit $?
fi

# -----------------------------------------------------------------
# 0. Gold Band agents. The registry's declarations are validated before
# anything is written: a refusal prints the crate's lines and stops the
# run, so setup never writes from a declaration the crate refuses, and a
# validation that could not run stops the run with its own message.
# -----------------------------------------------------------------
gold_band_validate

# -----------------------------------------------------------------
# 1. Command-line tools.
# -----------------------------------------------------------------
tokens_and_keys

package_proxy_setup "${HOME:-}"

log "command-line tools"

# Claude Code CLI.
if [ "${CLOUD}" = 1 ]; then
  log "claude-code: not set up in cloud mode"
else
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
# `hermes doctor` / `hermes skills list` against a local config.
if command -v hermes >/dev/null 2>&1; then
  run_sh 'uv tool upgrade --exclude-newer "7 days" '\''hermes-agent[mcp]'\'''
  ok "hermes $(hermes --version 2>/dev/null | head -1)"
else
  run_sh 'uv tool install --exclude-newer "7 days" --python '\''>=3.11,<3.14'\'' '\''hermes-agent[mcp]'\'''
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "hermes would be installed"
  elif command -v hermes >/dev/null 2>&1; then
    ok "hermes installed"
    new_binaries+=("hermes")
  else
    fail "hermes still not on PATH after install; open a new shell and re-run"
  fi
fi

# Mistral Vibe, the agent for personal notes. Gets the estate MCP
# servers, skills and AGENTS.md pointer like Claude Code and
# Codex; its MCP servers are merged into ~/.vibe/config.toml from
# registry/generated/vibe/mcp-servers.toml.
if command -v vibe >/dev/null 2>&1; then
  run_sh 'uv tool upgrade --exclude-newer "7 days" mistral-vibe'
  ok "vibe $(vibe --version 2>/dev/null | head -1)"
else
  run_sh 'uv tool install --exclude-newer "7 days" --python '\''>=3.12'\'' mistral-vibe'
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "vibe would be installed"
  elif command -v vibe >/dev/null 2>&1; then
    ok "vibe installed"
    new_binaries+=("vibe")
  else
    fail "vibe still not on PATH after install; open a new shell and re-run"
  fi
fi

# Google Antigravity CLI (`agy`). Gets the workstation MCP fleet
# minus Private Memory, the agent-kit instructions pointer and the
# kit's skills.
if [ "${CLOUD}" = 1 ]; then
  log "antigravity-cli: not set up in cloud mode"
else
  if [ "$(uname -s)" = Darwin ]; then
    if brew list --cask antigravity-cli >/dev/null 2>&1; then
      run brew upgrade --cask antigravity-cli
      ok "agy $(agy --version 2>/dev/null | head -1)"
    else
      run brew install --cask antigravity-cli
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "antigravity-cli would be installed"
      elif brew list --cask antigravity-cli >/dev/null 2>&1; then
        ok "antigravity-cli installed"
        new_binaries+=("agy")
      else
        fail "antigravity-cli: brew install --cask antigravity-cli did not install it; is Homebrew on PATH?"
      fi
    fi
  else
    warn "antigravity-cli: macOS only; skipped on $(uname -s)"
  fi
fi

# Google Antigravity, the desktop agent manager. Same
# configuration as the CLI: one global MCP file, one global rules
# file, and the kit's skills in each of the two global skills
# folders.
if [ "${CLOUD}" = 1 ]; then
  log "antigravity: not set up in cloud mode"
else
  if [ "$(uname -s)" = Darwin ]; then
    if brew list --cask antigravity >/dev/null 2>&1; then
      run brew upgrade --cask antigravity
      ok "antigravity present"
    else
      run brew install --cask antigravity
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "antigravity would be installed"
      elif brew list --cask antigravity >/dev/null 2>&1; then
        ok "antigravity installed"
      else
        fail "antigravity: brew install --cask antigravity did not install it; is Homebrew on PATH?"
      fi
    fi
  else
    warn "antigravity: macOS only; skipped on $(uname -s)"
  fi
fi

# OpenCode 2, the open-source terminal coding agent. Gets the
# workstation MCP fleet minus Private Memory, the agent-kit
# instructions pointer, the kit's and the vendored skills, and the
# skills of every enabled Claude Code plugin. Not a council or
# critique target.
if [ "${CLOUD}" = 1 ]; then
  log "opencode: not set up in cloud mode"
else
  if command -v opencode >/dev/null 2>&1; then
    run_sh '! brew list --formula opencode >/dev/null 2>&1 || brew upgrade opencode'
    ok "opencode $(opencode --version 2>/dev/null | head -1)"
  else
    run_sh 'if command -v brew >/dev/null 2>&1; then brew install opencode; else echo "opencode: Homebrew is not on PATH; skipped (install Homebrew or OpenCode 2, then re-run)" >&2; fi'
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "opencode would be installed"
    elif command -v opencode >/dev/null 2>&1; then
      ok "opencode installed"
      new_binaries+=("opencode")
    else
      fail "opencode still not on PATH after install; open a new shell and re-run"
    fi
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
if nix_provides uv; then
  ok "uv: provided by Nix; setup does not install or update it"
  if copy=$(non_nix_copy uv); then
    warn "uv: ${copy} is not from Nix and can shadow the Nix-provided uv; remove it"
  fi
else
  if command -v uv >/dev/null 2>&1; then
    run_sh 'uv self update 2>/dev/null || brew upgrade uv 2>/dev/null || true'
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
fi

report_antigravity_signin

# -----------------------------------------------------------------
# Gold Band v0.19.1 (987fc3e): built from an existing
# clone through the crate gate, Linux only. No clone: skipped, said so.
# -----------------------------------------------------------------
gold_band_setup() {
  local rc=0 mode=()
  if [ "$(uname -s)" != Linux ]; then
    log "gold band: the Mac build is not part of this step, skipped on $(uname -s)"
    return 0
  fi
  if [ "${CLOUD}" = 1 ]; then
    log "gold band: not built in a cloud session"
    return 0
  fi
  if [ ! -e "${HOME}/Projects/diodeme/Gold-Band/.git" ]; then
    log "gold band: skipped, no clone at ${HOME}/Projects/diodeme/Gold-Band"
    return 0
  fi
  [ "${CHECK_ONLY}" = 1 ] && mode=(--check)
  GOLD_BAND_DIR="${HOME}/Projects/diodeme/Gold-Band" GOLD_BAND_REF="987fc3e298b15d725a7f0e9b609b0f77dba63a74" \
    bash "${KIT_ROOT}/scripts/gold-band/build-linux.sh" "${mode[@]}" || rc=$?
  if [ "${rc}" = 0 ]; then
    ok "gold band: v0.19.1 (987fc3e) is handled, see the lines above"
  elif [ "${rc}" = 3 ]; then
    fail "gold band: a crate in its Cargo.lock is younger than 7 days or unprovable; the build was stopped"
  else
    fail "gold band: the build failed (exit ${rc})"
  fi
}
gold_band_setup

# -----------------------------------------------------------------
# 2. Claude Code profiles.
#
# A profile is a config root: CLAUDE_CONFIG_DIR moves credentials AND
# conversation history together, so a second login is a second root.
# The primary keeps the default location; every secondary shares the
# surfaces named in the registry by symlinking back into it, and keeps
# its own projects/, sessions/, history.jsonl and .claude.json.
# -----------------------------------------------------------------
# work: The default profile a bare `claude` uses, and the home of every shared asset the other profiles link back to.
CLAUDE_PROFILE_DIRS=("${CLAUDE_HOME}")

if [ "${DO_PROFILES}" = 1 ]; then
  log "claude profiles"
  ok "${CLAUDE_HOME} (primary)"

  # The personal-account profile. Same skills, plugins, LSPs and
  # MCP fleet as work; its own login and its own conversation
  # history.
  profile_dir="$HOME/.claude-personal"
  if [ "${profile_dir}" = "${CLAUDE_HOME}" ] || [ "${profile_dir}" -ef "${CLAUDE_HOME}" ]; then
    fail "personal: is the work profile directory ${profile_dir}; point it elsewhere"
  else
    if [ "${CHECK_ONLY}" = 1 ] && [ ! -d "${profile_dir}" ]; then
      log "would create ${profile_dir}"
    else
      mkdir -p "${profile_dir}"
    fi
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "skills"
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "agents"
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "commands"
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "hooks"
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "plugins"
    link_profile_path "${CLAUDE_HOME}" "${profile_dir}" "settings.json"
    CLAUDE_PROFILE_DIRS+=("${profile_dir}")
    install_profile_launcher "personal" "${profile_dir}"

    if [ -s "${profile_dir}/.claude.json" ]; then
      ok "personal: ${profile_dir} is set up"
    else
      log "personal: log in with  claude-personal  (its own account)"
    fi
  fi
else
  log "secondary claude profiles skipped (--no-profiles)"
fi

# Claude Code may not read or run anything that touches Private Memory's
# files: the Vibe homes (their session logs) and the gated token. Against accidents,
# not a determined agent; the passphrase is the boundary.
claude_private_denies

# MCP servers the registry trusts to run every tool without a prompt.
claude_mcp_allows

# Claude Code credits neither itself nor its model in commits or PRs.
claude_no_attribution

# On NixOS rustup arrives from Nix with no default toolchain: cargo fails
# until it is set, with or without Claude Code on this machine.
rustup_default_toolchain

# Before anything touches a Vibe home (MCP merge, converge, skills), and
# whether or not vibe is on PATH yet.
[ "${CLOUD}" = 1 ] || _vibe_split_homes

# -----------------------------------------------------------------
# 3. Claude Code marketplaces and plugins.
#
# Installed into the PRIMARY profile only. plugins/ is a shared
# surface and enabledPlugins lives in the shared settings.json, so a
# second install per profile would write the same state twice.
# -----------------------------------------------------------------
if ! command -v claude >/dev/null 2>&1; then
  fail "claude is not on PATH; skipping plugins, language servers and MCP"
else
  log "plugin marketplaces"
  ensure_marketplace claude-plugins-official anthropics/claude-plugins-official 43fd1f9db0941b2b83cb62268e35970d4a67c202 agent-kit-official
  ensure_marketplace caveman JuliusBrussee/caveman 8b0c1d3699b8d83e87fe4605b378da20c41555e0
  ensure_marketplace drawio jgraph/drawio-mcp 14b318b19cc37b159f841227b9d11fbd18ce18ea
  ensure_marketplace kubernetes-skill LukasNiessen/kubernetes-skill 34f93c1e409156958cc7c410ab1aa88ab512f10f
  if [ "${CLOUD}" = 1 ]; then
    log "ai-software-architect: not set up in cloud mode"
  else
    ensure_marketplace ai-software-architect codenamev/ai-software-architect 73d6f9a63effc9aba948f5ea5a29fd73e65f547b
  fi

  log "plugins"
  installed_plugins="$(claude plugin list --json 2>/dev/null || true)"
  # caveman: Compressed output mode plus the cavecrew subagents.
  case " ${unpinned_markets[*]-} " in *" caveman "*)
    warn "caveman: not installed: its marketplace is not at its pinned commit" ;;
  *)
    if run claude plugin install caveman@caveman --yes --scope user; then
      run claude plugin enable caveman@caveman >/dev/null 2>&1 || true
      ok "caveman@caveman installed and enabled"
    else
      warn "caveman@caveman install failed"
    fi
    ;;
  esac
  # drawio: Native .drawio diagram authoring and export.
  case " ${unpinned_markets[*]-} " in *" drawio "*)
    warn "drawio: not installed: its marketplace is not at its pinned commit" ;;
  *)
    if run claude plugin install drawio@drawio --yes --scope user; then
      run claude plugin enable drawio@drawio >/dev/null 2>&1 || true
      ok "drawio@drawio installed and enabled"
    else
      warn "drawio@drawio install failed"
    fi
    ;;
  esac
  # frontend-design: Visual design guidance for new UI work.
  case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
    warn "frontend-design: not installed: its marketplace is not at its pinned commit" ;;
  *)
    case "${installed_plugins-}" in *'"frontend-design@claude-plugins-official"'*)
      run claude plugin uninstall frontend-design@claude-plugins-official >/dev/null 2>&1 \
        && ok "frontend-design: removed the copy installed as frontend-design@claude-plugins-official" ;;
    esac
    if run claude plugin install frontend-design@agent-kit-official --yes --scope user; then
      run claude plugin enable frontend-design@agent-kit-official >/dev/null 2>&1 || true
      ok "frontend-design@agent-kit-official installed and enabled"
    else
      warn "frontend-design@agent-kit-official install failed"
    fi
    ;;
  esac
  # mattpocock-skills: TDD, code review, domain modelling, grilling.
  case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
    warn "mattpocock-skills: not installed: its marketplace is not at its pinned commit" ;;
  *)
    case "${installed_plugins-}" in *'"mattpocock-skills@claude-plugins-official"'*)
      run claude plugin uninstall mattpocock-skills@claude-plugins-official >/dev/null 2>&1 \
        && ok "mattpocock-skills: removed the copy installed as mattpocock-skills@claude-plugins-official" ;;
    esac
    if run claude plugin install mattpocock-skills@agent-kit-official --yes --scope user; then
      run claude plugin enable mattpocock-skills@agent-kit-official >/dev/null 2>&1 || true
      ok "mattpocock-skills@agent-kit-official installed and enabled"
    else
      warn "mattpocock-skills@agent-kit-official install failed"
    fi
    ;;
  esac
  # security-guidance: Security review of generated code. Ships its own PreToolUse and Stop hooks. Those are the PLUGIN's hooks, not estate hooks — the retired knowledge hooks are unrelated and the purge step matches on the
  case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
    warn "security-guidance: not installed: its marketplace is not at its pinned commit" ;;
  *)
    case "${installed_plugins-}" in *'"security-guidance@claude-plugins-official"'*)
      run claude plugin uninstall security-guidance@claude-plugins-official >/dev/null 2>&1 \
        && ok "security-guidance: removed the copy installed as security-guidance@claude-plugins-official" ;;
    esac
    if run claude plugin install security-guidance@agent-kit-official --yes --scope user; then
      run claude plugin enable security-guidance@agent-kit-official >/dev/null 2>&1 || true
      ok "security-guidance@agent-kit-official installed and enabled"
    else
      warn "security-guidance@agent-kit-official install failed"
    fi
    ;;
  esac
  # kubernetes-skill: Manifest and cluster failure-mode guidance.
  case " ${unpinned_markets[*]-} " in *" kubernetes-skill "*)
    warn "kubernetes-skill: not installed: its marketplace is not at its pinned commit" ;;
  *)
    if run claude plugin install kubernetes-skill@kubernetes-skill --yes --scope user; then
      run claude plugin enable kubernetes-skill@kubernetes-skill >/dev/null 2>&1 || true
      ok "kubernetes-skill@kubernetes-skill installed and enabled"
    else
      warn "kubernetes-skill@kubernetes-skill install failed"
    fi
    ;;
  esac
  # github: GitHub MCP server. Needs a token; it fails to connect with "Authorization header is badly formatted" when the credential is absent or malformed, which reads like a missing server rather than a missing
  if [ "${CLOUD}" = 1 ]; then
    log "github: not set up in cloud mode"
  else
    case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
      warn "github: not installed: its marketplace is not at its pinned commit" ;;
    *)
      case "${installed_plugins-}" in *'"github@claude-plugins-official"'*)
        run claude plugin uninstall github@claude-plugins-official >/dev/null 2>&1 \
          && ok "github: removed the copy installed as github@claude-plugins-official" ;;
      esac
      if run claude plugin install github@agent-kit-official --yes --scope user; then
        run claude plugin enable github@agent-kit-official >/dev/null 2>&1 || true
        ok "github@agent-kit-official installed and enabled"
      else
        warn "github@agent-kit-official install failed"
      fi
      ;;
    esac
  fi
  # ai-software-architect: Architecture review personas. Off by default — large surface, rarely the right tool.
  if [ "${CLOUD}" = 1 ]; then
    log "ai-software-architect: not set up in cloud mode"
  else
    case " ${unpinned_markets[*]-} " in *" ai-software-architect "*)
      warn "ai-software-architect: not installed: its marketplace is not at its pinned commit" ;;
    *)
      if run claude plugin install ai-software-architect@ai-software-architect --yes --scope user; then
        run claude plugin disable ai-software-architect@ai-software-architect >/dev/null 2>&1 || true
        ok "ai-software-architect@ai-software-architect installed, left disabled on purpose"
        disabled_plugins+=("ai-software-architect: disabled on purpose in registry")
      else
        warn "ai-software-architect@ai-software-architect install failed"
      fi
      ;;
    esac
  fi

  hindsight_coding_agents
  # memory-private on|off|status [DIR]: keeps a directory's sessions out of
  # Shared Memory (HINDSIGHT_DISABLED=true in its local settings).
  install_kit_command memory-private
  # agent-kit-private: the passphrase gate in front of the Private Memory token.
  install_kit_command agent-kit-private
  # agent-kit-secret: the one secret store (Keychain on macOS, libsecret on Linux).
  install_kit_command agent-kit-secret
  # kubernetes-mcp-readonly: the kubernetes MCP server, pinned to the read-only kube context.
  install_kit_command kubernetes-mcp-readonly
  # The Vibe launchers. Plain vibe is Vibe itself: the shared Vibe home, the
  # personal login, Shared Memory. vibe-study is the same home with the Study
  # account; vibe-private has its own home and Private Memory, behind the gate.
  # Workstation only: a cloud session gets plain vibe.
  if [ "${CLOUD}" != 1 ]; then
    install_kit_command vibe-study
    install_kit_command vibe-private
    # Antigravity's MCP config cannot hold a token, so Shared Memory reaches
    # it through this stdio proxy, which reads the token from the keychain.
    if [ "${ANTIGRAVITY_READY}" = 1 ]; then
      install_kit_command antigravity-mcp-proxy
    fi
  fi
  # Retired: Vibe saves to memory by tool now, not every turn.
  retire_kit_command vibe-retain-private
  retire_port_forward_agents

  # ---------------------------------------------------------------
  # 4. Plugin drift detection.
  #
  # Each marketplace is checked out at its pinned commit, which decides
  # the commit every plugin in it should be at. A plugin installed before
  # (or off) its pin is updated to it; only one that will not move is
  # reported.
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

    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/caveman" caveman 8b0c1d3699b8d83e87fe4605b378da20c41555e0)"
    installed_commit="$(installed_plugin_commit "${manifest}" "caveman@caveman")"
    if [ -z "${expected_commit}" ]; then
      warn "caveman: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "caveman: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "caveman: at its pinned commit"
    else
      run claude plugin update caveman@caveman >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "caveman@caveman")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "caveman: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "caveman: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("caveman: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/drawio" drawio 14b318b19cc37b159f841227b9d11fbd18ce18ea)"
    installed_commit="$(installed_plugin_commit "${manifest}" "drawio@drawio")"
    if [ -z "${expected_commit}" ]; then
      warn "drawio: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "drawio: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "drawio: at its pinned commit"
    else
      run claude plugin update drawio@drawio >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "drawio@drawio")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "drawio: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "drawio: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("drawio: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/claude-plugins-official" frontend-design 43fd1f9db0941b2b83cb62268e35970d4a67c202)"
    installed_commit="$(installed_plugin_commit "${manifest}" "frontend-design@agent-kit-official")"
    if [ -z "${expected_commit}" ]; then
      warn "frontend-design: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "frontend-design: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "frontend-design: at its pinned commit"
    else
      run claude plugin update frontend-design@agent-kit-official >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "frontend-design@agent-kit-official")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "frontend-design: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "frontend-design: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("frontend-design: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/claude-plugins-official" mattpocock-skills 43fd1f9db0941b2b83cb62268e35970d4a67c202)"
    installed_commit="$(installed_plugin_commit "${manifest}" "mattpocock-skills@agent-kit-official")"
    if [ -z "${expected_commit}" ]; then
      warn "mattpocock-skills: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "mattpocock-skills: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "mattpocock-skills: at its pinned commit"
    else
      run claude plugin update mattpocock-skills@agent-kit-official >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "mattpocock-skills@agent-kit-official")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "mattpocock-skills: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "mattpocock-skills: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("mattpocock-skills: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/claude-plugins-official" security-guidance 43fd1f9db0941b2b83cb62268e35970d4a67c202)"
    installed_commit="$(installed_plugin_commit "${manifest}" "security-guidance@agent-kit-official")"
    if [ -z "${expected_commit}" ]; then
      warn "security-guidance: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "security-guidance: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "security-guidance: at its pinned commit"
    else
      run claude plugin update security-guidance@agent-kit-official >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "security-guidance@agent-kit-official")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "security-guidance: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "security-guidance: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("security-guidance: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/kubernetes-skill" kubernetes-skill 34f93c1e409156958cc7c410ab1aa88ab512f10f)"
    installed_commit="$(installed_plugin_commit "${manifest}" "kubernetes-skill@kubernetes-skill")"
    if [ -z "${expected_commit}" ]; then
      warn "kubernetes-skill: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "kubernetes-skill: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "kubernetes-skill: at its pinned commit"
    else
      run claude plugin update kubernetes-skill@kubernetes-skill >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "kubernetes-skill@kubernetes-skill")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "kubernetes-skill: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "kubernetes-skill: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("kubernetes-skill: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/claude-plugins-official" github 43fd1f9db0941b2b83cb62268e35970d4a67c202)"
    installed_commit="$(installed_plugin_commit "${manifest}" "github@agent-kit-official")"
    if [ -z "${expected_commit}" ]; then
      warn "github: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "github: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "github: at its pinned commit"
    else
      run claude plugin update github@agent-kit-official >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "github@agent-kit-official")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "github: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "github: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("github: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
    expected_commit="$(expected_plugin_commit "${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}/ai-software-architect" ai-software-architect 73d6f9a63effc9aba948f5ea5a29fd73e65f547b)"
    installed_commit="$(installed_plugin_commit "${manifest}" "ai-software-architect@ai-software-architect")"
    if [ -z "${expected_commit}" ]; then
      warn "ai-software-architect: its marketplace entry pins no commit, so it cannot be pinned"
    elif [ -z "${installed_commit}" ]; then
      log "ai-software-architect: no commit recorded; drift not checked"
    elif [ "${installed_commit}" = "${expected_commit}" ]; then
      ok "ai-software-architect: at its pinned commit"
    else
      run claude plugin update ai-software-architect@ai-software-architect >/dev/null 2>&1 || true
      installed_commit="$(installed_plugin_commit "${manifest}" "ai-software-architect@ai-software-architect")"
      if [ "${installed_commit}" = "${expected_commit}" ]; then
        ok "ai-software-architect: moved to its pinned commit ${expected_commit:0:12}"
      else
        warn "ai-software-architect: installed ${installed_commit:0:12} will not move to its pinned ${expected_commit:0:12}"
        plugin_drift+=("ai-software-architect: ${installed_commit:0:12} vs ${expected_commit:0:12}")
      fi
    fi
  fi

  # ---------------------------------------------------------------
  # 5. Language servers: the plugin AND the binary it drives.
  #
  # An LSP plugin with no binary on PATH registers no tools and says
  # nothing about it. Install the plugin, but leave it disabled until
  # the binary is on PATH. This enables it on a second run once the
  # binary is installed.
  # ---------------------------------------------------------------
  if [ "${DO_LSP}" = 1 ]; then
    log "language servers"

    # typescript-lsp -> typescript-language-server (typescript, javascript)
    case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
      warn "typescript-lsp: not installed: its marketplace is not at its pinned commit" ;;
    *)
      case "${installed_plugins-}" in *'"typescript-lsp@claude-plugins-official"'*)
        run claude plugin uninstall typescript-lsp@claude-plugins-official >/dev/null 2>&1 \
          && ok "typescript-lsp: removed the copy installed as typescript-lsp@claude-plugins-official" ;;
      esac
      if run claude plugin install typescript-lsp@agent-kit-official --yes --scope user; then
        if command -v typescript-language-server >/dev/null 2>&1; then
          run claude plugin enable typescript-lsp@agent-kit-official >/dev/null 2>&1 || true
          ok "typescript-lsp: typescript-language-server on PATH, plugin enabled"
          if nix_provides typescript-language-server; then
            if copy=$(non_nix_copy typescript-language-server); then
              warn "typescript-lsp: ${copy} is not from Nix and can shadow the Nix-provided typescript-language-server; remove it"
            fi
          elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
            copy=$(non_nix_copy typescript-language-server) || copy=""
            warn "typescript-lsp: typescript-language-server on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
          fi
        else
          run claude plugin disable typescript-lsp@agent-kit-official >/dev/null 2>&1 || true
          lsp_cmd='npm install -g typescript-language-server typescript'
          run_sh "${lsp_cmd}" || true
          if command -v typescript-language-server >/dev/null 2>&1; then
            run claude plugin enable typescript-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "typescript-lsp: typescript-language-server installed and enabled"
          else
            warn "typescript-lsp: typescript-language-server is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("typescript-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "typescript-lsp@agent-kit-official install failed"
      fi
      ;;
    esac

    # pyright-lsp -> pyright-langserver (python)
    case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
      warn "pyright-lsp: not installed: its marketplace is not at its pinned commit" ;;
    *)
      case "${installed_plugins-}" in *'"pyright-lsp@claude-plugins-official"'*)
        run claude plugin uninstall pyright-lsp@claude-plugins-official >/dev/null 2>&1 \
          && ok "pyright-lsp: removed the copy installed as pyright-lsp@claude-plugins-official" ;;
      esac
      if run claude plugin install pyright-lsp@agent-kit-official --yes --scope user; then
        if command -v pyright-langserver >/dev/null 2>&1; then
          run claude plugin enable pyright-lsp@agent-kit-official >/dev/null 2>&1 || true
          ok "pyright-lsp: pyright-langserver on PATH, plugin enabled"
          if nix_provides pyright-langserver; then
            if copy=$(non_nix_copy pyright-langserver); then
              warn "pyright-lsp: ${copy} is not from Nix and can shadow the Nix-provided pyright-langserver; remove it"
            fi
          elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
            copy=$(non_nix_copy pyright-langserver) || copy=""
            warn "pyright-lsp: pyright-langserver on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
          fi
        else
          run claude plugin disable pyright-lsp@agent-kit-official >/dev/null 2>&1 || true
          lsp_cmd='npm install -g pyright'
          run_sh "${lsp_cmd}" || true
          if command -v pyright-langserver >/dev/null 2>&1; then
            run claude plugin enable pyright-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "pyright-lsp: pyright-langserver installed and enabled"
          else
            warn "pyright-lsp: pyright-langserver is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("pyright-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "pyright-lsp@agent-kit-official install failed"
      fi
      ;;
    esac

    # kotlin-lsp -> kotlin-lsp (kotlin)
    case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
      warn "kotlin-lsp: not installed: its marketplace is not at its pinned commit" ;;
    *)
      case "${installed_plugins-}" in *'"kotlin-lsp@claude-plugins-official"'*)
        run claude plugin uninstall kotlin-lsp@claude-plugins-official >/dev/null 2>&1 \
          && ok "kotlin-lsp: removed the copy installed as kotlin-lsp@claude-plugins-official" ;;
      esac
      if run claude plugin install kotlin-lsp@agent-kit-official --yes --scope user; then
        if command -v kotlin-lsp >/dev/null 2>&1; then
          run claude plugin enable kotlin-lsp@agent-kit-official >/dev/null 2>&1 || true
          ok "kotlin-lsp: kotlin-lsp on PATH, plugin enabled"
          if nix_provides kotlin-lsp; then
            if copy=$(non_nix_copy kotlin-lsp); then
              warn "kotlin-lsp: ${copy} is not from Nix and can shadow the Nix-provided kotlin-lsp; remove it"
            fi
          elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
            copy=$(non_nix_copy kotlin-lsp) || copy=""
            warn "kotlin-lsp: kotlin-lsp on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
          fi
        else
          run claude plugin disable kotlin-lsp@agent-kit-official >/dev/null 2>&1 || true
          if [ "$(uname -s)" = Linux ]; then
            lsp_cmd='v=263.7084.0 sum=ae0c4b01e17ab73a4a5227a78ea99e6dcbe3e499600315b22759a4e4cddd11df && curl -fsSL -o /tmp/kotlin-lsp.tgz "https://download.jetbrains.com/language-server/kotlin-server/${v}/kotlin-server-${v}.tar.gz" && echo "${sum}  /tmp/kotlin-lsp.tgz" | sha256sum -c - && rm -rf "${AK_OPT_DIR}/kotlin-lsp" && mkdir -p "${AK_OPT_DIR}/kotlin-lsp" "${AK_BIN_DIR}" && tar -xzf /tmp/kotlin-lsp.tgz -C "${AK_OPT_DIR}/kotlin-lsp" --strip-components=1 && rm -f /tmp/kotlin-lsp.tgz && ln -sf "${AK_OPT_DIR}/kotlin-lsp/kotlin-lsp.sh" "${AK_BIN_DIR}/kotlin-lsp"'
          else
            lsp_cmd='brew install kotlin-lsp'
          fi
          run_sh "${lsp_cmd}" || true
          if command -v kotlin-lsp >/dev/null 2>&1; then
            run claude plugin enable kotlin-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "kotlin-lsp: kotlin-lsp installed and enabled"
          else
            warn "kotlin-lsp: kotlin-lsp is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("kotlin-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "kotlin-lsp@agent-kit-official install failed"
      fi
      ;;
    esac

    # jdtls-lsp -> jdtls (java)
    case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
      warn "jdtls-lsp: not installed: its marketplace is not at its pinned commit" ;;
    *)
      case "${installed_plugins-}" in *'"jdtls-lsp@claude-plugins-official"'*)
        run claude plugin uninstall jdtls-lsp@claude-plugins-official >/dev/null 2>&1 \
          && ok "jdtls-lsp: removed the copy installed as jdtls-lsp@claude-plugins-official" ;;
      esac
      if run claude plugin install jdtls-lsp@agent-kit-official --yes --scope user; then
        if command -v jdtls >/dev/null 2>&1 && command -v java >/dev/null 2>&1; then
          run claude plugin enable jdtls-lsp@agent-kit-official >/dev/null 2>&1 || true
          ok "jdtls-lsp: jdtls on PATH, plugin enabled"
          if nix_provides jdtls; then
            if copy=$(non_nix_copy jdtls); then
              warn "jdtls-lsp: ${copy} is not from Nix and can shadow the Nix-provided jdtls; remove it"
            fi
          elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
            copy=$(non_nix_copy jdtls) || copy=""
            warn "jdtls-lsp: jdtls on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
          fi
        elif command -v jdtls >/dev/null 2>&1; then
          run claude plugin disable jdtls-lsp@agent-kit-official >/dev/null 2>&1 || true
          warn "jdtls-lsp: jdtls is on PATH but java is not; plugin left disabled"
          missing_lsp_binaries+=("jdtls-lsp: install java, which jdtls runs")
        else
          run claude plugin disable jdtls-lsp@agent-kit-official >/dev/null 2>&1 || true
          if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
            warn "jdtls-lsp: jdtls is absent; on NixOS declare pkgs.jdt-language-server and a java in nix-config; plugin left disabled"
            missing_lsp_binaries+=("jdtls-lsp: declare pkgs.jdt-language-server and a java in nix-config")
          else
            if [ "$(uname -s)" = Linux ]; then
              lsp_cmd='curl -fsSL -o /tmp/jdtls.tgz https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz && rm -rf "${AK_OPT_DIR}/jdtls" && mkdir -p "${AK_OPT_DIR}/jdtls" "${AK_BIN_DIR}" && tar -xzf /tmp/jdtls.tgz -C "${AK_OPT_DIR}/jdtls" && rm -f /tmp/jdtls.tgz && ln -sf "${AK_OPT_DIR}/jdtls/bin/jdtls" "${AK_BIN_DIR}/jdtls"'
            else
              lsp_cmd='brew install jdtls'
            fi
            run_sh "${lsp_cmd}" || true
            if command -v jdtls >/dev/null 2>&1 && command -v java >/dev/null 2>&1; then
              run claude plugin enable jdtls-lsp@agent-kit-official >/dev/null 2>&1 || true
              ok "jdtls-lsp: jdtls installed and enabled"
            else
              warn "jdtls-lsp: jdtls is absent; plugin left disabled (install missing: ${lsp_cmd})"
              missing_lsp_binaries+=("jdtls-lsp: install with: ${lsp_cmd}")
            fi
          fi
        fi
      else
        warn "jdtls-lsp@agent-kit-official install failed"
      fi
      ;;
    esac

    # ruby-lsp -> ruby-lsp (ruby)
    if [ "${CLOUD}" = 1 ]; then
      log "ruby-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "ruby-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"ruby-lsp@claude-plugins-official"'*)
          run claude plugin uninstall ruby-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "ruby-lsp: removed the copy installed as ruby-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install ruby-lsp@agent-kit-official --yes --scope user; then
          if command -v ruby-lsp >/dev/null 2>&1; then
            run claude plugin enable ruby-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "ruby-lsp: ruby-lsp on PATH, plugin enabled"
            if nix_provides ruby-lsp; then
              if copy=$(non_nix_copy ruby-lsp); then
                warn "ruby-lsp: ${copy} is not from Nix and can shadow the Nix-provided ruby-lsp; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy ruby-lsp) || copy=""
              warn "ruby-lsp: ruby-lsp on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable ruby-lsp@agent-kit-official >/dev/null 2>&1 || true
            if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              warn "ruby-lsp: ruby-lsp is absent; on NixOS declare pkgs.ruby-lsp in nix-config; plugin left disabled"
              missing_lsp_binaries+=("ruby-lsp: declare pkgs.ruby-lsp in nix-config")
            else
              lsp_cmd='gem install ruby-lsp'
              run_sh "${lsp_cmd}" || true
              if command -v ruby-lsp >/dev/null 2>&1; then
                run claude plugin enable ruby-lsp@agent-kit-official >/dev/null 2>&1 || true
                ok "ruby-lsp: ruby-lsp installed and enabled"
              else
                warn "ruby-lsp: ruby-lsp is absent; plugin left disabled (install missing: ${lsp_cmd})"
                missing_lsp_binaries+=("ruby-lsp: install with: ${lsp_cmd}")
              fi
            fi
          fi
        else
          warn "ruby-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # gopls-lsp -> gopls (go)
    if [ "${CLOUD}" = 1 ]; then
      log "gopls-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "gopls-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"gopls-lsp@claude-plugins-official"'*)
          run claude plugin uninstall gopls-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "gopls-lsp: removed the copy installed as gopls-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install gopls-lsp@agent-kit-official --yes --scope user; then
          if command -v gopls >/dev/null 2>&1; then
            run claude plugin enable gopls-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "gopls-lsp: gopls on PATH, plugin enabled"
            if nix_provides gopls; then
              if copy=$(non_nix_copy gopls); then
                warn "gopls-lsp: ${copy} is not from Nix and can shadow the Nix-provided gopls; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy gopls) || copy=""
              warn "gopls-lsp: gopls on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable gopls-lsp@agent-kit-official >/dev/null 2>&1 || true
            if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              warn "gopls-lsp: gopls is absent; on NixOS declare pkgs.gopls in nix-config; plugin left disabled"
              missing_lsp_binaries+=("gopls-lsp: declare pkgs.gopls in nix-config")
            else
              lsp_cmd='go install golang.org/x/tools/gopls@latest'
              run_sh "${lsp_cmd}" || true
              if command -v gopls >/dev/null 2>&1; then
                run claude plugin enable gopls-lsp@agent-kit-official >/dev/null 2>&1 || true
                ok "gopls-lsp: gopls installed and enabled"
              else
                warn "gopls-lsp: gopls is absent; plugin left disabled (install missing: ${lsp_cmd})"
                missing_lsp_binaries+=("gopls-lsp: install with: ${lsp_cmd}")
              fi
            fi
          fi
        else
          warn "gopls-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # rust-analyzer-lsp -> rust-analyzer (rust)
    if [ "${CLOUD}" = 1 ]; then
      log "rust-analyzer-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "rust-analyzer-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"rust-analyzer-lsp@claude-plugins-official"'*)
          run claude plugin uninstall rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "rust-analyzer-lsp: removed the copy installed as rust-analyzer-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install rust-analyzer-lsp@agent-kit-official --yes --scope user; then
          if command -v rust-analyzer >/dev/null 2>&1; then
            run claude plugin enable rust-analyzer-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "rust-analyzer-lsp: rust-analyzer on PATH, plugin enabled"
            if nix_provides rust-analyzer; then
              if copy=$(non_nix_copy rust-analyzer); then
                warn "rust-analyzer-lsp: ${copy} is not from Nix and can shadow the Nix-provided rust-analyzer; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy rust-analyzer) || copy=""
              warn "rust-analyzer-lsp: rust-analyzer on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable rust-analyzer-lsp@agent-kit-official >/dev/null 2>&1 || true
            if ! command -v rustup >/dev/null 2>&1; then
              log "rust-analyzer-lsp: no rustup on this machine; plugin left off"
            else
              if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
                warn "rust-analyzer-lsp: rust-analyzer is absent; on NixOS declare pkgs.rust-analyzer in nix-config; plugin left disabled"
                missing_lsp_binaries+=("rust-analyzer-lsp: declare pkgs.rust-analyzer in nix-config")
              else
                lsp_cmd='rustup component add rust-analyzer'
                run_sh "${lsp_cmd}" || true
                if command -v rust-analyzer >/dev/null 2>&1; then
                  run claude plugin enable rust-analyzer-lsp@agent-kit-official >/dev/null 2>&1 || true
                  ok "rust-analyzer-lsp: rust-analyzer installed and enabled"
                else
                  warn "rust-analyzer-lsp: rust-analyzer is absent; plugin left disabled (install missing: ${lsp_cmd})"
                  missing_lsp_binaries+=("rust-analyzer-lsp: install with: ${lsp_cmd}")
                fi
              fi
            fi
          fi
        else
          warn "rust-analyzer-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # lua-lsp -> lua-language-server (lua)
    if [ "${CLOUD}" = 1 ]; then
      log "lua-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "lua-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"lua-lsp@claude-plugins-official"'*)
          run claude plugin uninstall lua-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "lua-lsp: removed the copy installed as lua-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install lua-lsp@agent-kit-official --yes --scope user; then
          if command -v lua-language-server >/dev/null 2>&1; then
            run claude plugin enable lua-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "lua-lsp: lua-language-server on PATH, plugin enabled"
            if nix_provides lua-language-server; then
              if copy=$(non_nix_copy lua-language-server); then
                warn "lua-lsp: ${copy} is not from Nix and can shadow the Nix-provided lua-language-server; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy lua-language-server) || copy=""
              warn "lua-lsp: lua-language-server on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable lua-lsp@agent-kit-official >/dev/null 2>&1 || true
            if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              warn "lua-lsp: lua-language-server is absent; on NixOS declare pkgs.lua-language-server in nix-config; plugin left disabled"
              missing_lsp_binaries+=("lua-lsp: declare pkgs.lua-language-server in nix-config")
            else
              lsp_cmd='brew install lua-language-server'
              run_sh "${lsp_cmd}" || true
              if command -v lua-language-server >/dev/null 2>&1; then
                run claude plugin enable lua-lsp@agent-kit-official >/dev/null 2>&1 || true
                ok "lua-lsp: lua-language-server installed and enabled"
              else
                warn "lua-lsp: lua-language-server is absent; plugin left disabled (install missing: ${lsp_cmd})"
                missing_lsp_binaries+=("lua-lsp: install with: ${lsp_cmd}")
              fi
            fi
          fi
        else
          warn "lua-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # clangd-lsp -> clangd (c, cpp)
    if [ "${CLOUD}" = 1 ]; then
      log "clangd-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "clangd-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"clangd-lsp@claude-plugins-official"'*)
          run claude plugin uninstall clangd-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "clangd-lsp: removed the copy installed as clangd-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install clangd-lsp@agent-kit-official --yes --scope user; then
          if command -v clangd >/dev/null 2>&1; then
            run claude plugin enable clangd-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "clangd-lsp: clangd on PATH, plugin enabled"
            if nix_provides clangd; then
              if copy=$(non_nix_copy clangd); then
                warn "clangd-lsp: ${copy} is not from Nix and can shadow the Nix-provided clangd; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy clangd) || copy=""
              warn "clangd-lsp: clangd on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable clangd-lsp@agent-kit-official >/dev/null 2>&1 || true
            if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              warn "clangd-lsp: clangd is absent; on NixOS declare pkgs.clang-tools in nix-config; plugin left disabled"
              missing_lsp_binaries+=("clangd-lsp: declare pkgs.clang-tools in nix-config")
            else
              lsp_cmd='brew install llvm'
              run_sh "${lsp_cmd}" || true
              if command -v clangd >/dev/null 2>&1; then
                run claude plugin enable clangd-lsp@agent-kit-official >/dev/null 2>&1 || true
                ok "clangd-lsp: clangd installed and enabled"
              else
                warn "clangd-lsp: clangd is absent; plugin left disabled (install missing: ${lsp_cmd})"
                missing_lsp_binaries+=("clangd-lsp: install with: ${lsp_cmd}")
              fi
            fi
          fi
        else
          warn "clangd-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # php-lsp -> intelephense (php)
    if [ "${CLOUD}" = 1 ]; then
      log "php-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "php-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"php-lsp@claude-plugins-official"'*)
          run claude plugin uninstall php-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "php-lsp: removed the copy installed as php-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install php-lsp@agent-kit-official --yes --scope user; then
          if command -v intelephense >/dev/null 2>&1; then
            run claude plugin enable php-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "php-lsp: intelephense on PATH, plugin enabled"
          else
            run claude plugin disable php-lsp@agent-kit-official >/dev/null 2>&1 || true
            lsp_cmd='npm install -g intelephense'
            run_sh "${lsp_cmd}" || true
            if command -v intelephense >/dev/null 2>&1; then
              run claude plugin enable php-lsp@agent-kit-official >/dev/null 2>&1 || true
              ok "php-lsp: intelephense installed and enabled"
            else
              warn "php-lsp: intelephense is absent; plugin left disabled (install missing: ${lsp_cmd})"
              missing_lsp_binaries+=("php-lsp: install with: ${lsp_cmd}")
            fi
          fi
        else
          warn "php-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # csharp-lsp -> csharp-ls (csharp)
    if [ "${CLOUD}" = 1 ]; then
      log "csharp-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "csharp-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"csharp-lsp@claude-plugins-official"'*)
          run claude plugin uninstall csharp-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "csharp-lsp: removed the copy installed as csharp-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install csharp-lsp@agent-kit-official --yes --scope user; then
          if command -v csharp-ls >/dev/null 2>&1; then
            run claude plugin enable csharp-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "csharp-lsp: csharp-ls on PATH, plugin enabled"
            if nix_provides csharp-ls; then
              if copy=$(non_nix_copy csharp-ls); then
                warn "csharp-lsp: ${copy} is not from Nix and can shadow the Nix-provided csharp-ls; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy csharp-ls) || copy=""
              warn "csharp-lsp: csharp-ls on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable csharp-lsp@agent-kit-official >/dev/null 2>&1 || true
            if ! command -v dotnet >/dev/null 2>&1; then
              log "csharp-lsp: no dotnet on this machine; plugin left off"
            else
              if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
                warn "csharp-lsp: csharp-ls is absent; on NixOS declare pkgs.csharp-ls in nix-config; plugin left disabled"
                missing_lsp_binaries+=("csharp-lsp: declare pkgs.csharp-ls in nix-config")
              else
                lsp_cmd='dotnet tool install --global csharp-ls'
                run_sh "${lsp_cmd}" || true
                if command -v csharp-ls >/dev/null 2>&1; then
                  run claude plugin enable csharp-lsp@agent-kit-official >/dev/null 2>&1 || true
                  ok "csharp-lsp: csharp-ls installed and enabled"
                else
                  warn "csharp-lsp: csharp-ls is absent; plugin left disabled (install missing: ${lsp_cmd})"
                  missing_lsp_binaries+=("csharp-lsp: install with: ${lsp_cmd}")
                fi
              fi
            fi
          fi
        else
          warn "csharp-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # swift-lsp -> sourcekit-lsp (swift)
    if [ "${CLOUD}" = 1 ]; then
      log "swift-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "swift-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"swift-lsp@claude-plugins-official"'*)
          run claude plugin uninstall swift-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "swift-lsp: removed the copy installed as swift-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install swift-lsp@agent-kit-official --yes --scope user; then
          if command -v sourcekit-lsp >/dev/null 2>&1; then
            run claude plugin enable swift-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "swift-lsp: sourcekit-lsp on PATH, plugin enabled"
            if nix_provides sourcekit-lsp; then
              if copy=$(non_nix_copy sourcekit-lsp); then
                warn "swift-lsp: ${copy} is not from Nix and can shadow the Nix-provided sourcekit-lsp; remove it"
              fi
            elif [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              copy=$(non_nix_copy sourcekit-lsp) || copy=""
              warn "swift-lsp: sourcekit-lsp on PATH is not from Nix; on NixOS it comes from nixos-modules' devWorkstation, so remove ${copy:-it} once that is deployed"
            fi
          else
            run claude plugin disable swift-lsp@agent-kit-official >/dev/null 2>&1 || true
            if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then
              warn "swift-lsp: sourcekit-lsp is absent; on NixOS declare pkgs.sourcekit-lsp in nix-config; plugin left disabled"
              missing_lsp_binaries+=("swift-lsp: declare pkgs.sourcekit-lsp in nix-config")
            else
              warn "swift-lsp: sourcekit-lsp is absent and ships with its platform toolchain; plugin left disabled"
              missing_lsp_binaries+=("swift-lsp: missing from toolchain (no install command)")
            fi
          fi
        else
          warn "swift-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi

    # liquid-lsp -> shopify (liquid)
    if [ "${CLOUD}" = 1 ]; then
      log "liquid-lsp: not set up in cloud mode"
    else
      case " ${unpinned_markets[*]-} " in *" claude-plugins-official "*)
        warn "liquid-lsp: not installed: its marketplace is not at its pinned commit" ;;
      *)
        case "${installed_plugins-}" in *'"liquid-lsp@claude-plugins-official"'*)
          run claude plugin uninstall liquid-lsp@claude-plugins-official >/dev/null 2>&1 \
            && ok "liquid-lsp: removed the copy installed as liquid-lsp@claude-plugins-official" ;;
        esac
        if run claude plugin install liquid-lsp@agent-kit-official --yes --scope user; then
          if command -v shopify >/dev/null 2>&1; then
            run claude plugin enable liquid-lsp@agent-kit-official >/dev/null 2>&1 || true
            ok "liquid-lsp: shopify on PATH, so plugin enabled (off by default)"
          else
            run claude plugin disable liquid-lsp@agent-kit-official >/dev/null 2>&1 || true
            log "liquid-lsp: off by default; install shopify to enable it"
          fi
        else
          warn "liquid-lsp@agent-kit-official install failed"
        fi
        ;;
      esac
    fi
  else
    log "language servers skipped (--no-lsp)"
  fi

  converge_plugins

  # ---------------------------------------------------------------
  # 6. MCP servers, registered for Claude Code at user scope.
  #
  # The registry is authoritative: declared servers are ensured, then
  # converge_mcp removes retired and undeclared ones (see CONTEXT.md).
  #
  # Every claude call here runs once per profile: a server lives in the
  # profile's own .claude.json, which is the one file two profiles must
  # not share, so the fleet is registered into each of them.
  # ---------------------------------------------------------------
  if [ "${DO_MCP}" = 1 ]; then
    log "MCP servers"

    # Hindsight long-term memory -- explicit read/write/search
    # knowledge tools.
    if [ -z "${HINDSIGHT_API_TOKEN:-}" ] && [ "${CLOUD}" != 1 ]; then
      log "memory-api: HINDSIGHT_API_TOKEN is not set; not registered (see tokens and keys)"
      skipped_mcp_servers+=("memory-api: export HINDSIGHT_API_TOKEN")
      skipped_mcp_names+=(memory-api)
    else
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user memory-api >/dev/null 2>&1 || true
        if run_redacted "claude mcp add memory-api" claude_each_profile claude mcp add --scope user memory-api --transport http https://hindsight-api.jorisjonkers.dev/mcp/default/ --header "Authorization: Bearer $(secret_ref HINDSIGHT_API_TOKEN)"; then
          ok "memory-api registered (claude)"
        else
          fail "memory-api registration failed (claude)"
        fi
      fi
      if command -v codex >/dev/null 2>&1; then
        run codex mcp remove memory-api >/dev/null 2>&1 || true
        if run_redacted "codex mcp add memory-api" codex mcp add memory-api --url https://hindsight-api.jorisjonkers.dev/mcp/default/ --bearer-token-env-var HINDSIGHT_API_TOKEN; then
          ok "memory-api registered (codex)"
        else
          fail "memory-api registration failed (codex)"
        fi
      fi
    fi

    # Private Memory, the private Hindsight bank -- personal notes.
    # On only in vibe-private, behind the passphrase gate; no other
    # agent or launcher gets it.
    if [ "${CLOUD}" = 1 ]; then
      log "memory-private: not set up in cloud mode"
    else
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user memory-private >/dev/null 2>&1 || true
      fi
      run rm -f "${XDG_CONFIG_HOME:-${HOME}/.config}/agent-kit/memory-private.mcp.json"
      if command -v codex >/dev/null 2>&1; then
        run codex mcp remove memory-private >/dev/null 2>&1 || true
      fi
    fi

    # n8n workflows through its instance-level MCP server.
    if [ "${CLOUD}" = 1 ]; then
      log "n8n: not set up in cloud mode"
    else
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user n8n >/dev/null 2>&1 || true
        if run_redacted "claude mcp add n8n" claude_each_profile claude mcp add --scope user n8n --transport http https://n8n.jorisjonkers.dev/mcp-server/http; then
          ok "n8n registered (claude)"
        else
          fail "n8n registration failed (claude)"
        fi
      fi
      if command -v codex >/dev/null 2>&1; then
        if run uv run --no-project --exclude-newer "7 days" --with tomlkit python "${KIT_ROOT}/scripts/codex-add-url-mcp.py" "${CODEX_HOME:-${HOME}/.codex}/config.toml" n8n https://n8n.jorisjonkers.dev/mcp-server/http \
          && run codex mcp get n8n >/dev/null; then
          ok "n8n registered (codex)"
        else
          fail "n8n registration failed (codex)"
        fi
      fi
      oauth_status n8n
    fi

    # n8n node docs and schemas, docs-only, no API key.
    if ! npm ls -g --depth=0 "n8n-mcp@2.101.0" >/dev/null 2>&1; then
      run_sh 'npm install -g "n8n-mcp@2.101.0"' || true
    fi
    command -v n8n-mcp >/dev/null 2>&1 \
      || warn "n8n-mcp: n8n-mcp is not on PATH; skipping"
    if command -v claude >/dev/null 2>&1; then
      run claude_each_profile claude mcp remove --scope user n8n-mcp >/dev/null 2>&1 || true
      if run_redacted "claude mcp add n8n-mcp" claude_each_profile claude mcp add --scope user n8n-mcp -- n8n-mcp; then
        ok "n8n-mcp registered (claude)"
      else
        fail "n8n-mcp registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove n8n-mcp >/dev/null 2>&1 || true
      if run_redacted "codex mcp add n8n-mcp" codex mcp add n8n-mcp -- n8n-mcp; then
        ok "n8n-mcp registered (codex)"
      else
        fail "n8n-mcp registration failed (codex)"
      fi
    fi

    # Read-only cluster diagnostics, run locally against the
    # workstation's read-only kube context.
    if [ "${CLOUD}" = 1 ]; then
      log "kubernetes: not set up in cloud mode"
    else
      if ! npm ls -g --depth=0 "kubernetes-mcp-server@0.1.1" >/dev/null 2>&1; then
        run_sh 'npm install -g "kubernetes-mcp-server@0.1.1"' || true
      fi
      command -v kubernetes-mcp-server >/dev/null 2>&1 \
        || warn "kubernetes: kubernetes-mcp-server is not on PATH; skipping"
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user kubernetes >/dev/null 2>&1 || true
        if run_redacted "claude mcp add kubernetes" claude_each_profile claude mcp add --scope user kubernetes -- kubernetes-mcp-readonly; then
          ok "kubernetes registered (claude)"
        else
          fail "kubernetes registration failed (claude)"
        fi
      fi
      if command -v codex >/dev/null 2>&1; then
        run codex mcp remove kubernetes >/dev/null 2>&1 || true
        if run_redacted "codex mcp add kubernetes" codex mcp add kubernetes -- kubernetes-mcp-readonly; then
          ok "kubernetes registered (codex)"
        else
          fail "kubernetes registration failed (codex)"
        fi
      fi
    fi

    # Grimoire, the D&D 5e table companion at
    # grimoire.jorisjonkers.dev, through its MCP server.
    if [ "${CLOUD}" = 1 ]; then
      log "grimoire: not set up in cloud mode"
    else
      if [ -z "${GRIMOIRE_API_TOKEN:-}" ] && [ "${CLOUD}" != 1 ]; then
        log "grimoire: GRIMOIRE_API_TOKEN is not set; not registered (see tokens and keys)"
        skipped_mcp_servers+=("grimoire: export GRIMOIRE_API_TOKEN")
        skipped_mcp_names+=(grimoire)
      else
        if command -v claude >/dev/null 2>&1; then
          run claude_each_profile claude mcp remove --scope user grimoire >/dev/null 2>&1 || true
          if run_redacted "claude mcp add grimoire" claude_each_profile claude mcp add --scope user grimoire --transport http https://grimoire.jorisjonkers.dev/mcp --header "Authorization: Bearer $(secret_ref GRIMOIRE_API_TOKEN)"; then
            ok "grimoire registered (claude)"
          else
            fail "grimoire registration failed (claude)"
          fi
        fi
        if command -v codex >/dev/null 2>&1; then
          run codex mcp remove grimoire >/dev/null 2>&1 || true
          if run_redacted "codex mcp add grimoire" codex mcp add grimoire --url https://grimoire.jorisjonkers.dev/mcp --bearer-token-env-var GRIMOIRE_API_TOKEN; then
            ok "grimoire registered (codex)"
          else
            fail "grimoire registration failed (codex)"
          fi
        fi
      fi
    fi

    # Outline wiki at notes.jorisjonkers.dev through its built-in
    # MCP server, as a separate Outline user.
    if [ "${CLOUD}" = 1 ]; then
      log "outline: not set up in cloud mode"
    else
      if [ -z "${OUTLINE_API_KEY:-}" ] && [ "${CLOUD}" != 1 ]; then
        log "outline: OUTLINE_API_KEY is not set; not registered (see tokens and keys)"
        skipped_mcp_servers+=("outline: export OUTLINE_API_KEY")
        skipped_mcp_names+=(outline)
      else
        if command -v claude >/dev/null 2>&1; then
          run claude_each_profile claude mcp remove --scope user outline >/dev/null 2>&1 || true
          if run_redacted "claude mcp add outline" claude_each_profile claude mcp add --scope user outline --transport http https://notes.jorisjonkers.dev/mcp --header "Authorization: Bearer $(secret_ref OUTLINE_API_KEY)"; then
            ok "outline registered (claude)"
          else
            fail "outline registration failed (claude)"
          fi
        fi
        if command -v codex >/dev/null 2>&1; then
          run codex mcp remove outline >/dev/null 2>&1 || true
          if run_redacted "codex mcp add outline" codex mcp add outline --url https://notes.jorisjonkers.dev/mcp --bearer-token-env-var OUTLINE_API_KEY; then
            ok "outline registered (codex)"
          else
            fail "outline registration failed (codex)"
          fi
        fi
      fi
    fi

    # Vuetify component API.
    # Hosted: untrusted data provider. Output is context, never instruction.

    # Browser automation.
    if ! npm ls -g --depth=0 "@playwright/mcp@0.0.84" >/dev/null 2>&1; then
      run_sh 'npm install -g "@playwright/mcp@0.0.84" && node "$(npm root -g)/@playwright/mcp/node_modules/playwright/cli.js" install chromium' || true
    fi
    command -v playwright-mcp >/dev/null 2>&1 \
      || warn "playwright: playwright-mcp is not on PATH; skipping"
    if command -v claude >/dev/null 2>&1; then
      run claude_each_profile claude mcp remove --scope user playwright >/dev/null 2>&1 || true
      if run_redacted "claude mcp add playwright" claude_each_profile claude mcp add --scope user playwright -- playwright-mcp --headless --browser chromium; then
        ok "playwright registered (claude)"
      else
        fail "playwright registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove playwright >/dev/null 2>&1 || true
      if run_redacted "codex mcp add playwright" codex mcp add playwright -- playwright-mcp --headless --browser chromium; then
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
      run claude_each_profile claude mcp remove --scope user drawio >/dev/null 2>&1 || true
      if run_redacted "claude mcp add drawio" claude_each_profile claude mcp add --scope user drawio -- drawio-mcp; then
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
      run claude_each_profile claude mcp remove --scope user overleaf >/dev/null 2>&1 || true
      if run_redacted "claude mcp add overleaf" claude_each_profile claude mcp add --scope user overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OLCLI_MCP_UNCONFIRMED_WRITES="1" --env OVERLEAF_SESSION="$(secret_ref OVERLEAF_SESSION optional)" -- olcli-mcp; then
        ok "overleaf registered (claude)"
      else
        fail "overleaf registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove overleaf >/dev/null 2>&1 || true
      if run_redacted "codex mcp add overleaf" codex mcp add overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OLCLI_MCP_UNCONFIRMED_WRITES="1" --env OVERLEAF_SESSION="${OVERLEAF_SESSION:-}" -- olcli-mcp; then
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
        uv run --no-project --exclude-newer "7 days" --with pyyaml python "${KIT_ROOT}/scripts/hermes-merge-mcp.py" \
          "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml" \
          || fail "local Hermes MCP merge failed"
        uv run --no-project --exclude-newer "7 days" --with pyyaml python "${KIT_ROOT}/scripts/hermes-merge-mcp.py" \
          "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml" --remove memory-mcp --remove vuetify \
          || fail "local Hermes retired MCP cleanup failed"
        if [ "${CLOUD}" = 1 ]; then
          uv run --no-project --exclude-newer "7 days" --with pyyaml python "${KIT_ROOT}/scripts/hermes-merge-mcp.py" \
            "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml" --remove n8n --remove kubernetes --remove grimoire --remove outline \
            || fail "local Hermes MCP cleanup failed"
        fi
      fi
    else
      warn "hermes not on PATH; local Hermes MCP config not merged"
    fi
    # Mistral Vibe reads [[mcp_servers]] from ${VIBE_HOME}/config.toml and has
    # no `mcp add`; the helper replaces the servers it manages, keeps the rest.
    # Two Vibe homes: the shared one (vibe, vibe-study) never holds Private
    # Memory; vibe-private's own home is the only one that does.
    if command -v vibe >/dev/null 2>&1 && [ "${VIBE_HOMES_READY}" = 1 ]; then
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "would merge Vibe MCP servers into ${VIBE_HOME}/config.toml"
      else
        vibe_remove=()
        [ "${CLOUD}" = 1 ] && vibe_remove+=(--remove n8n --remove kubernetes --remove grimoire --remove outline)
        uv run --no-project --exclude-newer "7 days" --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \
          "${KIT_ROOT}/registry/generated/vibe/mcp-servers.toml" --config "${VIBE_HOME}/config.toml" --remove memory-private ${vibe_remove[@]+"${vibe_remove[@]}"} \
          && ok "Vibe MCP servers merged" \
          || fail "Vibe MCP merge failed"
        _vibe_models "${VIBE_HOME}"
        vibe_hooks_remove=(--remove agent-kit-retain-private)
        _vibe_hooks "${VIBE_HOME}" ${vibe_hooks_remove[@]+"${vibe_hooks_remove[@]}"}
      fi
      _vibe_agents_pointer "${VIBE_HOME}" shared
      gateway_pointer "${VIBE_HOME}/AGENTS.md"
      attribution_rule "${VIBE_HOME}/AGENTS.md"
      vibe_no_commit_signature "${VIBE_HOME}"
      [ "${CLOUD}" = 1 ] || _vibe_private_home
    else
      warn "vibe not on PATH, or its home is not split yet; Vibe MCP config not merged"
    fi
    # Antigravity reads one global mcp_config.json, shared by the app, the CLI
    # and the IDE. `agy mcp add` writes one server at a time, so the fragment
    # is merged in one pass. It names the stdio proxy for any server that
    # needs a token, so no secret reaches that file. Private Memory is removed
    # even when someone added it by hand: no keep file keeps it here.
    if [ "${CLOUD}" != 1 ] && [ "${ANTIGRAVITY_READY}" = 1 ] && command -v agy >/dev/null 2>&1; then
      if [ "${CHECK_ONLY}" = 1 ]; then
        log "would merge Antigravity MCP servers into ${ANTIGRAVITY_HOME}/mcp_config.json"
      elif python3 "${KIT_ROOT}/scripts/antigravity-merge-mcp.py" \
        "${KIT_ROOT}/registry/generated/antigravity/mcp_config.json" --config "${ANTIGRAVITY_HOME}/mcp_config.json" --remove memory-mcp --remove vuetify --remove memory-private; then
        ok "Antigravity MCP servers merged"
      else
        fail "Antigravity MCP merge failed"
      fi
    fi
    # Antigravity's global rules file. Never ~/.gemini/AGENTS.md or
    # GEMINI.md: Gemini CLI reads those.
    if [ "${CLOUD}" != 1 ] && [ "${ANTIGRAVITY_READY}" = 1 ]; then
      _agents_pointer "${ANTIGRAVITY_HOME}/AGENTS.md" shared
      gateway_pointer "${ANTIGRAVITY_HOME}/AGENTS.md"
      attribution_rule "${ANTIGRAVITY_HOME}/AGENTS.md"
    fi
    # OpenCode 2 reads mcp.servers from its global opencode.json and fills in
    # {env:NAME} at load, so the fragment names each credential and holds
    # none. OpenCode 1 reads another shape, so a V1 binary is named, not fed.
    if [ "${OPENCODE_READY}" = 1 ] && command -v opencode >/dev/null 2>&1; then
      opencode_version="$(opencode --version 2>/dev/null | head -1)"
      # V2 prints `opencode v2.2.5`; match the last word, with or without its v.
      opencode_release="${opencode_version##* }"
      case "${opencode_release#v}" in
        1.*)
          OPENCODE_V1=1
          warn "opencode: ${opencode_version} is OpenCode 1; its MCP config is left alone; agent-kit configures OpenCode 2 (brew install opencode)" ;;
      esac
      if [ "${OPENCODE_V1}" = 1 ]; then
        :
      elif [ "${CHECK_ONLY}" = 1 ]; then
        log "would merge OpenCode MCP servers into ${OPENCODE_HOME}/opencode.json"
      elif python3 "${KIT_ROOT}/scripts/opencode-merge-mcp.py" \
        "${KIT_ROOT}/registry/generated/opencode/opencode.json" --config "${OPENCODE_HOME}/opencode.json" --remove memory-mcp --remove vuetify --remove memory-private; then
        ok "OpenCode MCP servers merged"
      else
        fail "OpenCode MCP merge failed"
      fi
    fi
    # OpenCode's global rules file; it reads ~/.claude/CLAUDE.md only without one.
    if [ "${OPENCODE_READY}" = 1 ]; then
      _agents_pointer "${OPENCODE_HOME}/AGENTS.md" shared
      attribution_rule "${OPENCODE_HOME}/AGENTS.md"
    fi
    converge_mcp
    # Verify the VALUE, not the exit codes: ask Claude what it
    # actually has, and check every expected server is present.
    if [ "${CHECK_ONLY}" != 1 ]; then
     # Once per profile: each one answers for its own .claude.json.
     for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
      registered=$(claude_in_profile "${profile_dir}" claude mcp list 2>/dev/null || true)
      # Check all expected servers are registered
      expected_servers=(memory-api n8n-mcp playwright drawio overleaf)
      if [ "${CLOUD}" != 1 ]; then
        expected_servers+=(n8n kubernetes grimoire outline)
      fi
      for want in "${expected_servers[@]}"; do
        case " ${skipped_mcp_names[*]-} " in *" ${want} "*) continue ;; esac
        case "${registered}" in
          *"${want}"*) ;;
          *) warn "MCP server ${want} is not registered in ${profile_dir}" ;;
        esac
      done
     done
    fi
  else
    log "MCP registration skipped (--no-mcp)"
  fi
fi

# -----------------------------------------------------------------
# The model gateway: the `model-gateway` command on PATH, and a pointer to it in
# each client's global instruction file. A gateway that is not declared on this
# surface is retired: its command and its pointers are removed, if setup wrote them.
# -----------------------------------------------------------------
install_kit_command model-gateway
gateway_pointer "${CLAUDE_HOME}/CLAUDE.md"
gateway_pointer "${CODEX_HOME}/AGENTS.md"
if [ "${CLOUD}" != 1 ]; then
  gateway_pointer "${HOME}/.hermes/SOUL.md"
  gateway_pointer "${XDG_CONFIG_HOME:-${HOME}/.config}/opencode/AGENTS.md"
fi

# -----------------------------------------------------------------
# The projects layout: its rule in each client's global instruction file.
# A cloud session has no ~/Projects, so it gets no rule. A layout that is
# not declared on this surface is retired: setup removes the blocks it wrote.
# -----------------------------------------------------------------
if [ "${CLOUD}" != 1 ]; then
  layout_pointer "${CLAUDE_HOME}/CLAUDE.md"
  layout_pointer "${CODEX_HOME}/AGENTS.md"
  layout_pointer "${HOME}/.hermes/SOUL.md"
  layout_pointer "${XDG_CONFIG_HOME:-${HOME}/.config}/opencode/AGENTS.md"
fi

# -----------------------------------------------------------------
# No agent attribution: the rule in each agent's global instruction file.
# Vibe, Antigravity and OpenCode get it with their other instructions above.
# -----------------------------------------------------------------
attribution_rule "${CLAUDE_HOME}/CLAUDE.md"
attribution_rule "${CODEX_HOME}/AGENTS.md"
if [ "${CLOUD}" != 1 ]; then
  attribution_rule "${HOME}/.hermes/SOUL.md"
fi
# -----------------------------------------------------------------
# 7. First-party skills that ship in this repository.
#
# Copied, not fetched. Multi-file skills, so the whole tree moves and
# the destination is replaced rather than merged -- a stale script left
# behind from an older version is worse than a missing one.
# -----------------------------------------------------------------
log "first-party skills"

# Compose a pull request in the repository's own house style:
# reads the PR template, mines recent merged PRs, and gates on an
# explicit confirmation before anything is pushed or created.
src="${KIT_ROOT}/skills/workstation/pr-composer"
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
  dest="${VIBE_HOME}/skills/pr-composer"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
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
  dest="${VIBE_PRIVATE_HOME}/skills/pr-composer"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
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
  dest="${ANTIGRAVITY_HOME}/skills/pr-composer"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
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
  dest="${ANTIGRAVITY_CLI_HOME}/skills/pr-composer"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
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
  dest="${OPENCODE_HOME}/skills/pr-composer"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
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

# Cross-model orchestrator (Claude + Codex) for large,
# decomposable work: triage, cross-model planning, grill, a design
# checkpoint, guarded fan-out, a review council, and resumable
# status/tail.
src="${KIT_ROOT}/skills/workstation/council"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "council: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/council"
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
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/council"
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
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/council"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/council"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/council"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/council"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/council"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "council -> ${dest}"
    else
      fail "council: ${dest} is incomplete after copy"
    fi
  fi
fi

# Consult the current memory platform before designing or changing
# behavior that may depend on prior captures, and capture durable
# lessons near task completion.
src="${KIT_ROOT}/skills/workstation/kb-first"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "kb-first: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/kb-first"
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
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/kb-first"
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
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/kb-first"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/kb-first"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/kb-first"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/kb-first"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/kb-first"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "kb-first -> ${dest}"
    else
      fail "kb-first: ${dest} is incomplete after copy"
    fi
  fi
fi

# Token-budget discipline: progressive disclosure, bounded recall,
# narrow MCP profiles, and prompt-cache-friendly instruction
# ordering.
src="${KIT_ROOT}/skills/workstation/token-economy"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "token-economy: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/token-economy"
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
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/token-economy"
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
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/token-economy"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/token-economy"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/token-economy"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/token-economy"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/token-economy"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "token-economy -> ${dest}"
    else
      fail "token-economy: ${dest} is incomplete after copy"
    fi
  fi
fi

# Checklist for configuring a Claude Code or Codex session: config
# layers, MCP servers and profiles, memory-file hygiene, and
# Claude/Codex parity.
src="${KIT_ROOT}/skills/workstation/agent-session-bootstrap"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "agent-session-bootstrap: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/agent-session-bootstrap"
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
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/agent-session-bootstrap"
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
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/agent-session-bootstrap"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/agent-session-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/agent-session-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/agent-session-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/agent-session-bootstrap"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "agent-session-bootstrap -> ${dest}"
    else
      fail "agent-session-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
fi

# The critique pipeline's shared files: the bundled runner, the
# prompt preamble, roles, lenses, finding schema and default
# routing that the critique skills drive.
src="${KIT_ROOT}/skills/workstation/critique"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "critique: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/critique"
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
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/critique"
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
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/critique"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/critique"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/critique"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/critique"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/critique"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique -> ${dest}"
    else
      fail "critique: ${dest} is incomplete after copy"
    fi
  fi
fi

# Critique a feature's decisions after grilling and before
# /to-spec: gap critics from other model families list the
# questions nobody asked, and the orchestrator asks the owner the
# real ones.
src="${KIT_ROOT}/skills/workstation/review-grill"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "review-grill: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/review-grill"
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
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/review-grill"
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
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/review-grill"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/review-grill"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/review-grill"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-grill"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/review-grill"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-grill -> ${dest}"
    else
      fail "review-grill: ${dest} is incomplete after copy"
    fi
  fi
fi

# Critique a spec after /to-spec: critics from other model
# families review it read-only, and the orchestrator rules on
# their findings.
src="${KIT_ROOT}/skills/workstation/review-spec"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "review-spec: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/review-spec"
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
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/review-spec"
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
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/review-spec"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/review-spec"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/review-spec"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-spec"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/review-spec"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-spec -> ${dest}"
    else
      fail "review-spec: ${dest} is incomplete after copy"
    fi
  fi
fi

# Report how each critique critic client performs per stage, from
# the ledger, ending with routing suggestions the owner acts on.
src="${KIT_ROOT}/skills/workstation/critique-stats"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "critique-stats: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/critique-stats"
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
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/critique-stats"
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
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/critique-stats"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/critique-stats"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/critique-stats"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/critique-stats"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/critique-stats"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-stats -> ${dest}"
    else
      fail "critique-stats: ${dest} is incomplete after copy"
    fi
  fi
fi

# Check a spec's tickets after /to-tickets and before the first
# /implement: the deterministic ticket checker, then one critic
# from another model family lints their structure read-only, and
# the orchestrator fixes the tickets and asks before changing
# scope.
src="${KIT_ROOT}/skills/workstation/review-tickets"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "review-tickets: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/review-tickets"
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
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/review-tickets"
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
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/review-tickets"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/review-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/review-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/review-tickets"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-tickets -> ${dest}"
    else
      fail "review-tickets: ${dest} is incomplete after copy"
    fi
  fi
fi

# Review one ticket's staged change before its commit, after its
# /code-review: risk triage decides, one critic from another model
# family reviews the diff, each bug finding is decided by running
# its failing test, and the summary says "do not commit" while an
# accepted critical finding is unfixed.
src="${KIT_ROOT}/skills/workstation/review-implementation"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "review-implementation: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/review-implementation"
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
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/review-implementation"
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
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/review-implementation"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/review-implementation"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/review-implementation"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-implementation"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/review-implementation"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-implementation -> ${dest}"
    else
      fail "review-implementation: ${dest} is incomplete after copy"
    fi
  fi
fi

# Review a repository's decision records on the owner's word:
# critics from other model families review the ADRs read-only, the
# orchestrator rules on their findings, makes only editorial
# fixes, and asks the owner about anything that would change a
# decision.
src="${KIT_ROOT}/skills/workstation/critique-adrs"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "critique-adrs: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/critique-adrs"
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
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/critique-adrs"
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
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/critique-adrs"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/critique-adrs"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/critique-adrs"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/critique-adrs"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/critique-adrs"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "critique-adrs -> ${dest}"
    else
      fail "critique-adrs: ${dest} is incomplete after copy"
    fi
  fi
fi

# Review the whole feature diff before the pull request, after all
# tickets of a spec landed: critics from two other model families
# read the spec, the tickets and the feature diff, each accepted
# finding becomes a follow-up ticket, and the report opens with
# the gate line.
src="${KIT_ROOT}/skills/workstation/review-pr"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "review-pr: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/review-pr"
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
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/review-pr"
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
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/review-pr"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/review-pr"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/review-pr"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-pr"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/review-pr"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "review-pr -> ${dest}"
    else
      fail "review-pr: ${dest} is incomplete after copy"
    fi
  fi
fi

# The critique pipeline's stepper: says in a few lines where the
# current feature stands (phase, stage runs, open questions, gate)
# and names the single next step, running a critique stage on the
# owner's word and naming the Pocock command otherwise.
src="${KIT_ROOT}/skills/workstation/flow"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "flow: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/flow"
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
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/flow"
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
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/flow"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/flow"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/flow"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/flow"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/flow"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "flow -> ${dest}"
    else
      fail "flow: ${dest} is incomplete after copy"
    fi
  fi
fi

# The build tracker for a spec being implemented: a state file
# with each ticket's status, rendered into one page with a summary
# strip, a dependency graph and a list per milestone.
src="${KIT_ROOT}/skills/workstation/build-tracker"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "build-tracker: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/build-tracker"
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
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/build-tracker"
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
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/build-tracker"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/build-tracker"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/build-tracker"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/build-tracker"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/build-tracker"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "build-tracker -> ${dest}"
    else
      fail "build-tracker: ${dest} is incomplete after copy"
    fi
  fi
fi

# Where clones and worktrees live on a workstation:
# ~/Projects/<owner>/<repo> and
# ~/Projects/<owner>/worktrees/<repo>/<name>, and the guard hook
# that denies a clone or worktree outside them.
src="${KIT_ROOT}/skills/workstation/projects-layout"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "projects-layout: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/projects-layout"
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
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/projects-layout"
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
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/projects-layout"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/projects-layout"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/projects-layout"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/projects-layout"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/projects-layout"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "projects-layout -> ${dest}"
    else
      fail "projects-layout: ${dest} is incomplete after copy"
    fi
  fi
fi

# Write or change a Project's Intent for deploy-kit 1.x, validate
# it with deploy-kit validate, wire its publishing and propose the
# estate participants.yml entry, reading the canonical docs live
# with gh.
src="${KIT_ROOT}/skills/workstation/estate-project-intent"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "estate-project-intent: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/estate-project-intent"
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
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/estate-project-intent"
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
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/estate-project-intent"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/estate-project-intent"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/estate-project-intent"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/estate-project-intent"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/estate-project-intent"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-project-intent -> ${dest}"
    else
      fail "estate-project-intent: ${dest} is incomplete after copy"
    fi
  fi
fi

# Answer whether a release landed from the estate compose run, the
# commit status and the deployment: landed only when they agree,
# otherwise mixed with each signal and its link.
src="${KIT_ROOT}/skills/workstation/estate-delivery-status"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "estate-delivery-status: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/estate-delivery-status"
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
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/estate-delivery-status"
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
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/estate-delivery-status"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/estate-delivery-status"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/estate-delivery-status"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/estate-delivery-status"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/estate-delivery-status"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-delivery-status -> ${dest}"
    else
      fail "estate-delivery-status: ${dest} is incomplete after copy"
    fi
  fi
fi

# Plan a pause, rollback or resume of one estate Project and hand
# the owner the exact gh workflow run command, reading the
# canonical docs and the pin live with gh. It never dispatches a
# workflow.
src="${KIT_ROOT}/skills/workstation/estate-rollback"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "estate-rollback: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/estate-rollback"
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
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/estate-rollback"
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
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/estate-rollback"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/estate-rollback"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/estate-rollback"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/estate-rollback"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/estate-rollback"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-rollback -> ${dest}"
    else
      fail "estate-rollback: ${dest} is incomplete after copy"
    fi
  fi
fi

# Start or audit a repository against the estate templates, shared
# workflows, Renovate preset, commons libraries and testing
# standards, reporting gaps with evidence and drafting issues only
# after the owner says yes.
src="${KIT_ROOT}/skills/workstation/estate-bootstrap"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "estate-bootstrap: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/estate-bootstrap"
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
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/estate-bootstrap"
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
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/estate-bootstrap"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/estate-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/estate-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/estate-bootstrap"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
  dest="${OPENCODE_HOME}/skills/estate-bootstrap"
  if [ "${OPENCODE_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "estate-bootstrap -> ${dest}"
    else
      fail "estate-bootstrap: ${dest} is incomplete after copy"
    fi
  fi
fi

# The model gateway's runner and shipped defaults: the clients on
# this machine, the client models each offers with their family,
# difficulty tier and capability notes, and the caps and
# thresholds a delegation is held to.
src="${KIT_ROOT}/skills/workstation/model-gateway"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "model-gateway: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/model-gateway"
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
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/model-gateway"
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
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/model-gateway"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/model-gateway"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/model-gateway"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/model-gateway"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "model-gateway -> ${dest}"
    else
      fail "model-gateway: ${dest} is incomplete after copy"
    fi
  fi
fi

# Plan work to hand to other model clients through the model
# gateway: rate each task's difficulty tier, split it, pick the
# cheapest cost class whose model covers the tier, send one batch
# and verify what comes back, with few Claude tokens.
src="${KIT_ROOT}/skills/workstation/delegate"
if [ ! -f "${src}/SKILL.md" ]; then
  fail "delegate: ${src}/SKILL.md is missing; nothing to install"
else
  dest="${CLAUDE_HOME}/skills/delegate"
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
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
  dest="${CODEX_HOME}/skills/delegate"
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
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_HOME}/skills/delegate"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
  dest="${VIBE_PRIVATE_HOME}/skills/delegate"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_HOME}/skills/delegate"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
  dest="${ANTIGRAVITY_CLI_HOME}/skills/delegate"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would replace ${dest}"
  else
    rm -rf "${dest}"
    mkdir -p "$(dirname "${dest}")"
    cp -R "${src}" "${dest}"
    # Verify the value: the copy is only useful if SKILL.md and
    # every script actually landed.
    if [ -f "${dest}/SKILL.md" ] \
       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then
      ok "delegate -> ${dest}"
    else
      fail "delegate: ${dest} is incomplete after copy"
    fi
  fi
fi

# Retired first-party skills (registry retired_skills:): a renamed skill's
# old directory is removed from every agent home, not left beside the new one.
log "retired skills"
if [ -d "${CLAUDE_HOME}/skills/critique-decisions" ]; then
  dest="${CLAUDE_HOME}/skills/critique-decisions"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${CODEX_HOME}/skills/critique-decisions" ]; then
  dest="${CODEX_HOME}/skills/critique-decisions"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${VIBE_HOME}/skills/critique-decisions" ]; then
  dest="${VIBE_HOME}/skills/critique-decisions"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${VIBE_PRIVATE_HOME}/skills/critique-decisions" ]; then
  dest="${VIBE_PRIVATE_HOME}/skills/critique-decisions"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_HOME}/skills/critique-decisions" ]; then
  dest="${ANTIGRAVITY_HOME}/skills/critique-decisions"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_CLI_HOME}/skills/critique-decisions" ]; then
  dest="${ANTIGRAVITY_CLI_HOME}/skills/critique-decisions"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${OPENCODE_HOME}/skills/critique-decisions" ]; then
  dest="${OPENCODE_HOME}/skills/critique-decisions"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-decisions from ${dest}"
  fi
fi
if [ -d "${CLAUDE_HOME}/skills/critique-spec" ]; then
  dest="${CLAUDE_HOME}/skills/critique-spec"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${CODEX_HOME}/skills/critique-spec" ]; then
  dest="${CODEX_HOME}/skills/critique-spec"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${VIBE_HOME}/skills/critique-spec" ]; then
  dest="${VIBE_HOME}/skills/critique-spec"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${VIBE_PRIVATE_HOME}/skills/critique-spec" ]; then
  dest="${VIBE_PRIVATE_HOME}/skills/critique-spec"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_HOME}/skills/critique-spec" ]; then
  dest="${ANTIGRAVITY_HOME}/skills/critique-spec"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_CLI_HOME}/skills/critique-spec" ]; then
  dest="${ANTIGRAVITY_CLI_HOME}/skills/critique-spec"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${OPENCODE_HOME}/skills/critique-spec" ]; then
  dest="${OPENCODE_HOME}/skills/critique-spec"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill critique-spec from ${dest}"
  fi
fi
if [ -d "${CLAUDE_HOME}/skills/check-tickets" ]; then
  dest="${CLAUDE_HOME}/skills/check-tickets"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${CODEX_HOME}/skills/check-tickets" ]; then
  dest="${CODEX_HOME}/skills/check-tickets"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${VIBE_HOME}/skills/check-tickets" ]; then
  dest="${VIBE_HOME}/skills/check-tickets"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${VIBE_PRIVATE_HOME}/skills/check-tickets" ]; then
  dest="${VIBE_PRIVATE_HOME}/skills/check-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_HOME}/skills/check-tickets" ]; then
  dest="${ANTIGRAVITY_HOME}/skills/check-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_CLI_HOME}/skills/check-tickets" ]; then
  dest="${ANTIGRAVITY_CLI_HOME}/skills/check-tickets"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${OPENCODE_HOME}/skills/check-tickets" ]; then
  dest="${OPENCODE_HOME}/skills/check-tickets"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill check-tickets from ${dest}"
  fi
fi
if [ -d "${CLAUDE_HOME}/skills/review-ticket" ]; then
  dest="${CLAUDE_HOME}/skills/review-ticket"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${CODEX_HOME}/skills/review-ticket" ]; then
  dest="${CODEX_HOME}/skills/review-ticket"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${VIBE_HOME}/skills/review-ticket" ]; then
  dest="${VIBE_HOME}/skills/review-ticket"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${VIBE_PRIVATE_HOME}/skills/review-ticket" ]; then
  dest="${VIBE_PRIVATE_HOME}/skills/review-ticket"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_HOME}/skills/review-ticket" ]; then
  dest="${ANTIGRAVITY_HOME}/skills/review-ticket"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_CLI_HOME}/skills/review-ticket" ]; then
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-ticket"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${OPENCODE_HOME}/skills/review-ticket" ]; then
  dest="${OPENCODE_HOME}/skills/review-ticket"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-ticket from ${dest}"
  fi
fi
if [ -d "${CLAUDE_HOME}/skills/review-feature" ]; then
  dest="${CLAUDE_HOME}/skills/review-feature"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${CODEX_HOME}/skills/review-feature" ]; then
  dest="${CODEX_HOME}/skills/review-feature"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${VIBE_HOME}/skills/review-feature" ]; then
  dest="${VIBE_HOME}/skills/review-feature"
  if [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${VIBE_PRIVATE_HOME}/skills/review-feature" ]; then
  dest="${VIBE_PRIVATE_HOME}/skills/review-feature"
  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_HOME}/skills/review-feature" ]; then
  dest="${ANTIGRAVITY_HOME}/skills/review-feature"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${ANTIGRAVITY_CLI_HOME}/skills/review-feature" ]; then
  dest="${ANTIGRAVITY_CLI_HOME}/skills/review-feature"
  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then
    :
  elif [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi
if [ -d "${OPENCODE_HOME}/skills/review-feature" ]; then
  dest="${OPENCODE_HOME}/skills/review-feature"
  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would remove retired skill ${dest}"
  else
    rm -rf "${dest}"
    ok "removed retired skill review-feature from ${dest}"
  fi
fi

# The read-only Vibe critic agents, into the shared Vibe home only. Like
# its skills, a failed split leaves the home alone this run.
if [ "${VIBE_HOMES_READY}" = 1 ]; then
  log "Vibe critic agents"
  _vibe_critic_agents "${VIBE_HOME}"
fi

# The read-only Vibe delegate agents, into the shared Vibe home only.
if [ "${VIBE_HOMES_READY}" = 1 ]; then
  log "Vibe delegate agents"
  _vibe_delegate_agents "${VIBE_HOME}"
fi

# -----------------------------------------------------------------
# 8. Vendored third-party skills.
#
# Mirrors Hermes' sync-skills init container: clone the pinned ref,
# verify the commit sha (a mismatch is a supply-chain event, so it
# fails loudly rather than installing something else), then install
# every SKILL.md the source publishes into the Claude, Codex and
# OpenCode skills folders.
# -----------------------------------------------------------------
log "vendored skills"

# No plugin wraps this one on the workstation, unlike caveman,
# kubernetes-skill, drawio and mattpocock-skills.
# superpowers: https://github.com/obra/superpowers@v6.3.0
if [ "${CHECK_ONLY}" = 1 ]; then
  log "would clone https://github.com/obra/superpowers@v6.3.0 and install its skills (superpowers)"
else
  if ! command -v git >/dev/null 2>&1; then
    fail "superpowers: git is not on PATH; cannot vendor its skills"
  else
    vendor_stage="$(mktemp -d)"
    if git clone --quiet --depth 1 --branch 'v6.3.0' 'https://github.com/obra/superpowers' "${vendor_stage}" >/dev/null 2>&1 \
       && [ "$(git -C "${vendor_stage}" rev-parse HEAD 2>/dev/null)" = 'b36e0829c6d0140e93cfef2ca599b1b07d4a7797' ]; then
      vendor_skills_root="${vendor_stage}"
      [ -d "${vendor_stage}/skills" ] && vendor_skills_root="${vendor_stage}/skills"
      vendor_count=0
      while IFS= read -r vendor_skill_md; do
        vendor_dir="$(dirname "${vendor_skill_md}")"
        vendor_name="$(basename "${vendor_dir}")"
        vendor_homes=("${CLAUDE_HOME}" "${CODEX_HOME}")
        [ "${OPENCODE_READY}" = 1 ] && vendor_homes+=("${OPENCODE_HOME}")
        for vendor_home in "${vendor_homes[@]}"; do
          vendor_dest="${vendor_home}/skills/${vendor_name}"
          rm -rf "${vendor_dest}"
          mkdir -p "$(dirname "${vendor_dest}")"
          cp -R "${vendor_dir}" "${vendor_dest}"
        done
        vendor_count=$((vendor_count + 1))
      done < <(find "${vendor_skills_root}" -name SKILL.md)
      if [ "${vendor_count}" -gt 0 ]; then
        ok "superpowers: installed ${vendor_count} skill(s) from https://github.com/obra/superpowers@v6.3.0"
      else
        fail "superpowers: cloned https://github.com/obra/superpowers@v6.3.0 but found no SKILL.md under it"
      fi
    else
      fail "superpowers: could not clone https://github.com/obra/superpowers@v6.3.0 at commit b36e0829c6d0140e93cfef2ca599b1b07d4a7797 (supply-chain mismatch or network failure)"
    fi
    rm -rf "${vendor_stage}"
  fi
fi

# -----------------------------------------------------------------
# 8b. OpenCode: the skills of the enabled Claude Code plugins, copied
# from where Claude installed them. A plugin's hooks, agents and commands
# stay Claude's. Then the background service reloads, because it reads
# its config and skills once, at start.
# -----------------------------------------------------------------
if [ "${OPENCODE_READY}" = 1 ]; then
  log "OpenCode plugin skills"
  if command -v claude >/dev/null 2>&1; then
    opencode_skill_args=(--skills-dir "${OPENCODE_HOME}/skills" --plugins 'caveman@caveman,drawio@drawio,frontend-design@agent-kit-official,mattpocock-skills@agent-kit-official,security-guidance@agent-kit-official,kubernetes-skill@kubernetes-skill,github@agent-kit-official')
    [ "${CHECK_ONLY}" = 1 ] && opencode_skill_args+=(--check)
    opencode_skill_rc=0
    opencode_skill_out="$(python3 "${KIT_ROOT}/scripts/opencode-plugin-skills.py" \
      "${opencode_skill_args[@]}" 2>&1)" || opencode_skill_rc=$?
    while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${opencode_skill_out}"
    [ "${opencode_skill_rc}" = 0 ] || fail "OpenCode plugin skills did not all land (see above)"
  else
    warn "claude not on PATH; OpenCode gets no plugin skills"
  fi
  if [ "${CHECK_ONLY}" != 1 ] && command -v opencode >/dev/null 2>&1 \
     && opencode service status 2>/dev/null | grep -q '^http'; then
    if opencode service restart >/dev/null 2>&1; then
      ok "OpenCode service restarted"
    else
      warn "opencode service restart failed; restart it to load the new config"
    fi
  fi
fi

# -----------------------------------------------------------------
# 9. Agent hooks. Purge the retired ones (see purge_retired_hooks above),
# not just report them: a machine set up by the retired install.sh /
# install-agents.sh is exactly the one that still has these. Then merge
# in the critique hooks, after step 7 copied the scripts they run.
# -----------------------------------------------------------------
purge_retired_hooks
purge_leaked_runner_git_config
git_no_agent_attribution
purge_leaked_runner_workspace_trust /workspace
log "critique hooks"
critique_hooks install

if [ "${CLOUD}" = 1 ]; then
  cloud_finish
fi

# -----------------------------------------------------------------
# 10. Summary: what changed and what still needs attention.
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

# Report shared surfaces a secondary profile did not get
if [ "${#unshared_profile_paths[@]}" -gt 0 ]; then
  log "Profile surfaces not shared:"
  for entry in "${unshared_profile_paths[@]}"; do
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

# Final summary. What needs a hand is in the Still to do list below, once.
# Every item that needs a hand raised a warning or a failure, so these decide.
if [ "${failures}" = 0 ] && [ "${warnings}" = 0 ]; then
  log "Setup complete: everything is ready"
else
  log "Setup summary: ${failures} failure(s), ${warnings} warning(s)"
fi
print_todo

[ "${failures}" = 0 ]
