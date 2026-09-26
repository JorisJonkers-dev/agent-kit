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
#   ./setup-workstation.sh --cloud         a Claude Code cloud environment's
#                                          setup script (docs/CLOUD.md)
#   ./setup-workstation.sh --uninstall     remove what a PREVIOUS install.sh /
#                                          install-agents.sh install wrote,
#                                          then purge retired knowledge hooks
#                                          and exit -- does not touch anything
#                                          this script itself manages
#
# Run outside an agent-kit checkout (`curl | bash`), it fetches the
# published kit bundle and re-runs the copy inside it.
# AGENT_KIT_SKILLS_BUNDLE_URL overrides where that bundle comes from.
#
# Secrets are read from the environment and never written here:
#   HINDSIGHT_API_TOKEN  -> the memory-api MCP server
#   MEMORY_MCP_TOKEN  -> the memory-mcp MCP server
#   OVERLEAF_SESSION  -> the overleaf MCP server
#   HINDSIGHT_API_URL  -> the hindsight-memory plugin
#   HINDSIGHT_API_TOKEN  -> the hindsight-memory plugin

set -uo pipefail

CHECK_ONLY=0
DO_LSP=1
DO_MCP=1
DO_PROFILES=1
CLOUD=0
UNINSTALL=0
failures=0
warnings=0

# Track what needs attention after the run
skipped_mcp_servers=()     # MCP servers skipped due to missing credentials
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
    --cloud) CLOUD=1; DO_PROFILES=0 ;;
    --uninstall) UNINSTALL=1 ;;
    --help|-h) sed -n '2,34p' "$0"; exit 0 ;;
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
CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"

# Outside a checkout, re-run the copy in the release's kit bundle; --uninstall needs none of it.
use_kit_bundle() {
  [ -f "${KIT_ROOT}/registry/estate-tooling.yaml" ] && return 0
  if [ -n "${AGENT_KIT_BUNDLE_STAGE:-}" ]; then
    fail "the kit bundle at ${AGENT_KIT_BUNDLE_STAGE} is not a complete kit"
    exit 1
  fi
  local url="${AGENT_KIT_SKILLS_BUNDLE_URL:-https://assets.jorisjonkers.dev/agent-kit-skills.tar.gz}"
  local stage expected actual
  stage="$(mktemp -d)"
  log "not an agent-kit checkout; fetching the kit bundle from ${url}"
  if ! curl -fsSL -o "${stage}/kit.tar.gz" "${url}" \
     || ! curl -fsSL -o "${stage}/kit.tar.gz.sha256" "${url}.sha256"; then
    fail "kit bundle unavailable at ${url}; set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"
    exit 1
  fi
  expected="$(awk '{print $1}' "${stage}/kit.tar.gz.sha256")"
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
  AGENT_KIT_BUNDLE_STAGE="${stage}" exec bash "${stage}/installer/setup-workstation.sh" \
    ${ORIG_ARGS[@]+"${ORIG_ARGS[@]}"}
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

# ensure_port_forward: keeps the loopback of a workstation_connect MCP
# server alive (a launchd agent on macOS).
. "${KIT_ROOT}/installer/port-forward-agent.sh"

if [ "${CLOUD}" = 1 ]; then
  . "${KIT_ROOT}/installer/cloud-session.sh"
  cloud_prepare
fi

CLAUDE_HOOKS_DIR="${CLAUDE_HOME}/hooks"
CODEX_HOOKS_DIR="${CODEX_HOME}/hooks"

# -----------------------------------------------------------------
# The estate ships no hand-authored agent hooks. This purges the three
# retired knowledge-recall hook groups (edit recall, git-commit
# capture, session digest) from Claude settings.json by COMMAND
# BASENAME, so an operator's own hooks survive, and removes the now-dead
# hook script files themselves. Runs on every setup AND on --uninstall,
# because a machine set up by the retired install.sh/install-agents.sh
# is exactly the one that still has these.
# -----------------------------------------------------------------
purge_retired_hooks() {
  local settings="${CLAUDE_HOME}/settings.json"
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

  local stale
  for stale in \
    "${CLAUDE_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \
    "${CLAUDE_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \
    "${CLAUDE_HOOKS_DIR}/stop-session-digest.sh" \
    "${CLAUDE_HOOKS_DIR}/user-prompt-submit-recall.sh" \
    "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \
    "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \
    "${CODEX_HOOKS_DIR}/kb-stop-digest.sh" \
    "${CODEX_HOOKS_DIR}/kb-user-prompt-recall.sh" \
    "${CODEX_HOME}/hooks.json"
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
# --uninstall: remove every file the retired install.sh /
# install-agents.sh wrote (base skills, Spec Kit commands/skills, the
# knowledge-system version manifest) and the MCP entries they
# registered (knowledge, context7, vuetify), then purge retired hooks
# and exit. Does not touch anything this script itself manages -- a
# re-run with no flags reinstalls the current local_skills afterward.
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
  if command -v claude >/dev/null 2>&1; then
    run claude_each_profile claude mcp remove --scope user knowledge >/dev/null 2>&1 || true
    run claude_each_profile claude mcp remove --scope user context7 >/dev/null 2>&1 || true
    run claude_each_profile claude mcp remove --scope user vuetify >/dev/null 2>&1 || true
  fi
  if command -v codex >/dev/null 2>&1; then
    run codex mcp remove knowledge >/dev/null 2>&1 || true
    run codex mcp remove context7 >/dev/null 2>&1 || true
    run codex mcp remove vuetify >/dev/null 2>&1 || true
  fi
  purge_retired_hooks
  log "uninstall complete: ${failures} failure(s), ${warnings} warning(s)"
  [ "${failures}" = 0 ]
  exit $?
fi

# -----------------------------------------------------------------
# 1. Command-line tools.
# -----------------------------------------------------------------
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
# `hermes doctor` / `hermes skills list` against a local config;
# the in-cluster gateway runs the container image, not this.
if command -v hermes >/dev/null 2>&1; then
  run_sh 'uv tool upgrade '\''hermes-agent[mcp]'\'''
  ok "hermes $(hermes --version 2>/dev/null | head -1)"
else
  run_sh 'uv tool install --python '\''>=3.11,<3.14'\'' '\''hermes-agent[mcp]'\'''
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
    fail "personal: CLAUDE_CONFIG_DIR points at ${profile_dir}; re-run with it unset"
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
  if [ "${CLOUD}" = 1 ]; then
    log "ai-software-architect: not set up in cloud mode"
  else
    run claude plugin marketplace add codenamev/ai-software-architect 2>/dev/null \
      || run claude plugin marketplace update ai-software-architect >/dev/null 2>&1 \
      || warn "marketplace ai-software-architect (codenamev/ai-software-architect) could not be added or updated"
  fi
  run claude plugin marketplace add vectorize-io/hindsight 2>/dev/null \
    || run claude plugin marketplace update hindsight >/dev/null 2>&1 \
    || warn "marketplace hindsight (vectorize-io/hindsight) could not be added or updated"

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
  if [ "${CLOUD}" = 1 ]; then
    log "github: not set up in cloud mode"
  else
    if run claude plugin install github@claude-plugins-official --yes --scope user; then
      run claude plugin enable github@claude-plugins-official >/dev/null 2>&1 || true
      ok "github@claude-plugins-official installed and enabled"
    else
      warn "github@claude-plugins-official install failed"
    fi
  fi
  # ai-software-architect: Architecture review personas. Off by default — large surface, rarely the right tool.
  if [ "${CLOUD}" = 1 ]; then
    log "ai-software-architect: not set up in cloud mode"
  else
    if run claude plugin install ai-software-architect@ai-software-architect --yes --scope user; then
      run claude plugin disable ai-software-architect@ai-software-architect >/dev/null 2>&1 || true
      ok "ai-software-architect@ai-software-architect installed, left disabled on purpose"
      disabled_plugins+=("ai-software-architect: disabled on purpose in registry")
    else
      warn "ai-software-architect@ai-software-architect install failed"
    fi
  fi
  # hindsight-memory: Automatic Hindsight recall/retain for Claude Code via UserPromptSubmit/Stop hooks.
  if run claude plugin install hindsight-memory@hindsight --yes --scope user; then
    run claude plugin enable hindsight-memory@hindsight >/dev/null 2>&1 || true
    ok "hindsight-memory@hindsight installed and enabled"
  else
    warn "hindsight-memory@hindsight install failed"
  fi
  if [ -z "${HINDSIGHT_API_URL:-}" ]; then
    warn "hindsight-memory: HINDSIGHT_API_URL is not set (the estate Hindsight API, or it falls back to a personal local daemon); export it and re-run"
    missing_plugin_env+=("hindsight-memory: export HINDSIGHT_API_URL")
  fi
  if [ -z "${HINDSIGHT_API_TOKEN:-}" ]; then
    warn "hindsight-memory: HINDSIGHT_API_TOKEN is not set (per-host token from auth-api service-tokens (service MEMORY_API)); export it and re-run"
    missing_plugin_env+=("hindsight-memory: export HINDSIGHT_API_TOKEN")
  fi

  # ---------------------------------------------------------------
  # 4. Plugin drift detection.
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
    installed_commit="$(installed_plugin_commit "${manifest}" "hindsight-memory@hindsight")"
    if [ -z "${installed_commit}" ]; then
      log "hindsight-memory: no user-scope commit recorded; drift not checked"
    elif [ "${installed_commit}" != "16d4025f882ba232a2d4c72abd1eb47420e68e17" ]; then
      warn "hindsight-memory: installed ${installed_commit} differs from the registry's 16d4025f882b"
      plugin_drift+=("hindsight-memory: ${installed_commit} vs 16d4025f882b")
    else
      ok "hindsight-memory: at the expected commit"
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
    if run claude plugin install typescript-lsp@claude-plugins-official --yes --scope user; then
      if command -v typescript-language-server >/dev/null 2>&1; then
        run claude plugin enable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
        ok "typescript-lsp: typescript-language-server on PATH, plugin enabled"
      else
        run claude plugin disable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
        lsp_cmd='npm install -g typescript-language-server typescript'
        run_sh "${lsp_cmd}" || true
        if command -v typescript-language-server >/dev/null 2>&1; then
          run claude plugin enable typescript-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "typescript-lsp: typescript-language-server installed and enabled"
        else
          warn "typescript-lsp: typescript-language-server is absent; plugin left disabled (install missing: ${lsp_cmd})"
          missing_lsp_binaries+=("typescript-lsp: install with: ${lsp_cmd}")
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
        lsp_cmd='npm install -g pyright'
        run_sh "${lsp_cmd}" || true
        if command -v pyright-langserver >/dev/null 2>&1; then
          run claude plugin enable pyright-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "pyright-lsp: pyright-langserver installed and enabled"
        else
          warn "pyright-lsp: pyright-langserver is absent; plugin left disabled (install missing: ${lsp_cmd})"
          missing_lsp_binaries+=("pyright-lsp: install with: ${lsp_cmd}")
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
        if [ "$(uname -s)" = Linux ]; then
          lsp_cmd='v=263.4702.0 sum=1e11d2e5fefbf9ea215ad8dd6be95f2222897cd086e8cb7a661a52084a590405 && curl -fsSL -o /tmp/kotlin-lsp.tgz "https://download-cdn.jetbrains.com/language-server/kotlin-server/${v}/kotlin-server-${v}.tar.gz" && echo "${sum}  /tmp/kotlin-lsp.tgz" | sha256sum -c - && rm -rf "${AK_OPT_DIR}/kotlin-lsp" && mkdir -p "${AK_OPT_DIR}/kotlin-lsp" "${AK_BIN_DIR}" && tar -xzf /tmp/kotlin-lsp.tgz -C "${AK_OPT_DIR}/kotlin-lsp" --strip-components=1 && rm -f /tmp/kotlin-lsp.tgz && ln -sf "${AK_OPT_DIR}/kotlin-lsp/kotlin-lsp.sh" "${AK_BIN_DIR}/kotlin-lsp"'
        else
          lsp_cmd='brew install kotlin-lsp'
        fi
        run_sh "${lsp_cmd}" || true
        if command -v kotlin-lsp >/dev/null 2>&1; then
          run claude plugin enable kotlin-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "kotlin-lsp: kotlin-lsp installed and enabled"
        else
          warn "kotlin-lsp: kotlin-lsp is absent; plugin left disabled (install missing: ${lsp_cmd})"
          missing_lsp_binaries+=("kotlin-lsp: install with: ${lsp_cmd}")
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
        if [ "$(uname -s)" = Linux ]; then
          lsp_cmd='curl -fsSL -o /tmp/jdtls.tgz https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz && rm -rf "${AK_OPT_DIR}/jdtls" && mkdir -p "${AK_OPT_DIR}/jdtls" "${AK_BIN_DIR}" && tar -xzf /tmp/jdtls.tgz -C "${AK_OPT_DIR}/jdtls" && rm -f /tmp/jdtls.tgz && ln -sf "${AK_OPT_DIR}/jdtls/bin/jdtls" "${AK_BIN_DIR}/jdtls"'
        else
          lsp_cmd='brew install jdtls'
        fi
        run_sh "${lsp_cmd}" || true
        if command -v jdtls >/dev/null 2>&1; then
          run claude plugin enable jdtls-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "jdtls-lsp: jdtls installed and enabled"
        else
          warn "jdtls-lsp: jdtls is absent; plugin left disabled (install missing: ${lsp_cmd})"
          missing_lsp_binaries+=("jdtls-lsp: install with: ${lsp_cmd}")
        fi
      fi
    else
      warn "jdtls-lsp@claude-plugins-official install failed"
    fi

    # ruby-lsp -> ruby-lsp (ruby)
    if [ "${CLOUD}" = 1 ]; then
      log "ruby-lsp: not set up in cloud mode"
    else
      if run claude plugin install ruby-lsp@claude-plugins-official --yes --scope user; then
        if command -v ruby-lsp >/dev/null 2>&1; then
          run claude plugin enable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "ruby-lsp: ruby-lsp on PATH, plugin enabled"
        else
          run claude plugin disable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='gem install ruby-lsp'
          run_sh "${lsp_cmd}" || true
          if command -v ruby-lsp >/dev/null 2>&1; then
            run claude plugin enable ruby-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "ruby-lsp: ruby-lsp installed and enabled"
          else
            warn "ruby-lsp: ruby-lsp is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("ruby-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "ruby-lsp@claude-plugins-official install failed"
      fi
    fi

    # gopls-lsp -> gopls (go)
    if [ "${CLOUD}" = 1 ]; then
      log "gopls-lsp: not set up in cloud mode"
    else
      if run claude plugin install gopls-lsp@claude-plugins-official --yes --scope user; then
        if command -v gopls >/dev/null 2>&1; then
          run claude plugin enable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "gopls-lsp: gopls on PATH, plugin enabled"
        else
          run claude plugin disable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='go install golang.org/x/tools/gopls@latest'
          run_sh "${lsp_cmd}" || true
          if command -v gopls >/dev/null 2>&1; then
            run claude plugin enable gopls-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "gopls-lsp: gopls installed and enabled"
          else
            warn "gopls-lsp: gopls is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("gopls-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "gopls-lsp@claude-plugins-official install failed"
      fi
    fi

    # rust-analyzer-lsp -> rust-analyzer (rust)
    if [ "${CLOUD}" = 1 ]; then
      log "rust-analyzer-lsp: not set up in cloud mode"
    else
      if run claude plugin install rust-analyzer-lsp@claude-plugins-official --yes --scope user; then
        if command -v rust-analyzer >/dev/null 2>&1; then
          run claude plugin enable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "rust-analyzer-lsp: rust-analyzer on PATH, plugin enabled"
        else
          run claude plugin disable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='rustup component add rust-analyzer'
          run_sh "${lsp_cmd}" || true
          if command -v rust-analyzer >/dev/null 2>&1; then
            run claude plugin enable rust-analyzer-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "rust-analyzer-lsp: rust-analyzer installed and enabled"
          else
            warn "rust-analyzer-lsp: rust-analyzer is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("rust-analyzer-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "rust-analyzer-lsp@claude-plugins-official install failed"
      fi
    fi

    # lua-lsp -> lua-language-server (lua)
    if [ "${CLOUD}" = 1 ]; then
      log "lua-lsp: not set up in cloud mode"
    else
      if run claude plugin install lua-lsp@claude-plugins-official --yes --scope user; then
        if command -v lua-language-server >/dev/null 2>&1; then
          run claude plugin enable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "lua-lsp: lua-language-server on PATH, plugin enabled"
        else
          run claude plugin disable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='brew install lua-language-server'
          run_sh "${lsp_cmd}" || true
          if command -v lua-language-server >/dev/null 2>&1; then
            run claude plugin enable lua-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "lua-lsp: lua-language-server installed and enabled"
          else
            warn "lua-lsp: lua-language-server is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("lua-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "lua-lsp@claude-plugins-official install failed"
      fi
    fi

    # clangd-lsp -> clangd (c, cpp)
    if [ "${CLOUD}" = 1 ]; then
      log "clangd-lsp: not set up in cloud mode"
    else
      if run claude plugin install clangd-lsp@claude-plugins-official --yes --scope user; then
        if command -v clangd >/dev/null 2>&1; then
          run claude plugin enable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "clangd-lsp: clangd on PATH, plugin enabled"
        else
          run claude plugin disable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='brew install llvm'
          run_sh "${lsp_cmd}" || true
          if command -v clangd >/dev/null 2>&1; then
            run claude plugin enable clangd-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "clangd-lsp: clangd installed and enabled"
          else
            warn "clangd-lsp: clangd is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("clangd-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "clangd-lsp@claude-plugins-official install failed"
      fi
    fi

    # php-lsp -> intelephense (php)
    if [ "${CLOUD}" = 1 ]; then
      log "php-lsp: not set up in cloud mode"
    else
      if run claude plugin install php-lsp@claude-plugins-official --yes --scope user; then
        if command -v intelephense >/dev/null 2>&1; then
          run claude plugin enable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "php-lsp: intelephense on PATH, plugin enabled"
        else
          run claude plugin disable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='npm install -g intelephense'
          run_sh "${lsp_cmd}" || true
          if command -v intelephense >/dev/null 2>&1; then
            run claude plugin enable php-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "php-lsp: intelephense installed and enabled"
          else
            warn "php-lsp: intelephense is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("php-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "php-lsp@claude-plugins-official install failed"
      fi
    fi

    # csharp-lsp -> csharp-ls (csharp)
    if [ "${CLOUD}" = 1 ]; then
      log "csharp-lsp: not set up in cloud mode"
    else
      if run claude plugin install csharp-lsp@claude-plugins-official --yes --scope user; then
        if command -v csharp-ls >/dev/null 2>&1; then
          run claude plugin enable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "csharp-lsp: csharp-ls on PATH, plugin enabled"
        else
          run claude plugin disable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='dotnet tool install --global csharp-ls'
          run_sh "${lsp_cmd}" || true
          if command -v csharp-ls >/dev/null 2>&1; then
            run claude plugin enable csharp-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "csharp-lsp: csharp-ls installed and enabled"
          else
            warn "csharp-lsp: csharp-ls is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("csharp-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "csharp-lsp@claude-plugins-official install failed"
      fi
    fi

    # swift-lsp -> sourcekit-lsp (swift)
    if [ "${CLOUD}" = 1 ]; then
      log "swift-lsp: not set up in cloud mode"
    else
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
    fi

    # liquid-lsp -> shopify (liquid)
    if [ "${CLOUD}" = 1 ]; then
      log "liquid-lsp: not set up in cloud mode"
    else
      if run claude plugin install liquid-lsp@claude-plugins-official --yes --scope user; then
        if command -v shopify >/dev/null 2>&1; then
          run claude plugin enable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
          ok "liquid-lsp: shopify on PATH, plugin enabled (was disabled in registry)"
        else
          run claude plugin disable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
          lsp_cmd='npm install -g @shopify/cli'
          run_sh "${lsp_cmd}" || true
          if command -v shopify >/dev/null 2>&1; then
            run claude plugin enable liquid-lsp@claude-plugins-official >/dev/null 2>&1 || true
            ok "liquid-lsp: shopify installed and enabled"
          else
            warn "liquid-lsp: shopify is absent; plugin left disabled (install missing: ${lsp_cmd})"
            missing_lsp_binaries+=("liquid-lsp: install with: ${lsp_cmd}")
          fi
        fi
      else
        warn "liquid-lsp@claude-plugins-official install failed"
      fi
    fi
  else
    log "language servers skipped (--no-lsp)"
  fi

  # ---------------------------------------------------------------
  # 6. MCP servers, registered for Claude Code at user scope.
  #
  # The registry is authoritative: servers with `surfaces: []` or
  # `enabled: false` are removed. Others are ensured. Plugin-provided
  # servers (plugin:*:*) and hand-added ones are left untouched.
  #
  # Every claude call here runs once per profile: a server lives in the
  # profile's own .claude.json, which is the one file two profiles must
  # not share, so the fleet is registered into each of them.
  # ---------------------------------------------------------------
  if [ "${DO_MCP}" = 1 ]; then
    log "MCP servers"
    run claude_each_profile claude mcp remove --scope user memory >/dev/null 2>&1 || true
    log "memory: removed (retired in the registry)"
    run claude_each_profile claude mcp remove --scope user vuetify >/dev/null 2>&1 || true
    log "vuetify: removed (retired in the registry)"
    run claude_each_profile claude mcp remove --scope user knowledge >/dev/null 2>&1 || true
    log "knowledge: removed (retired in the registry)"


    # Hindsight long-term memory -- explicit read/write/search
    # knowledge tools.
    if [ -z "${HINDSIGHT_API_TOKEN:-}" ] && [ "${CLOUD}" != 1 ]; then
      warn "memory-api: HINDSIGHT_API_TOKEN is not set; skipping (export it and re-run)"
      skipped_mcp_servers+=("memory-api: export HINDSIGHT_API_TOKEN")
    else
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user memory-api >/dev/null 2>&1 || true
        if run_redacted "claude mcp add memory-api" claude_each_profile claude mcp add --scope user memory-api --transport http https://memory-api.jorisjonkers.dev/mcp --header "Authorization: Bearer $(secret_ref HINDSIGHT_API_TOKEN)"; then
          ok "memory-api registered (claude)"
        else
          fail "memory-api registration failed (claude)"
        fi
      fi
      if command -v codex >/dev/null 2>&1; then
        run codex mcp remove memory-api >/dev/null 2>&1 || true
        if run_redacted "codex mcp add memory-api" codex mcp add memory-api --url https://memory-api.jorisjonkers.dev/mcp --bearer-token-env-var HINDSIGHT_API_TOKEN; then
          ok "memory-api registered (codex)"
        else
          fail "memory-api registration failed (codex)"
        fi
      fi
    fi

    # Basic Memory -- shared Markdown notes with a semantic link
    # graph. Edit notes, never overwrite.
    if [ -z "${MEMORY_MCP_TOKEN:-}" ] && [ "${CLOUD}" != 1 ]; then
      warn "memory-mcp: MEMORY_MCP_TOKEN is not set; skipping (export it and re-run)"
      skipped_mcp_servers+=("memory-mcp: export MEMORY_MCP_TOKEN")
    else
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user memory-mcp >/dev/null 2>&1 || true
        if run_redacted "claude mcp add memory-mcp" claude_each_profile claude mcp add --scope user memory-mcp --transport http https://memory-mcp.jorisjonkers.dev/mcp --header "Authorization: Bearer $(secret_ref MEMORY_MCP_TOKEN)"; then
          ok "memory-mcp registered (claude)"
        else
          fail "memory-mcp registration failed (claude)"
        fi
      fi
      if command -v codex >/dev/null 2>&1; then
        run codex mcp remove memory-mcp >/dev/null 2>&1 || true
        if run_redacted "codex mcp add memory-mcp" codex mcp add memory-mcp --url https://memory-mcp.jorisjonkers.dev/mcp --bearer-token-env-var MEMORY_MCP_TOKEN; then
          ok "memory-mcp registered (codex)"
        else
          fail "memory-mcp registration failed (codex)"
        fi
      fi
    fi

    # Read-only cluster diagnostics through the server's own
    # ClusterRole.
    if [ "${CLOUD}" = 1 ]; then
      log "kubernetes: not set up in cloud mode"
    else

      # kubernetes reaches the cluster via a kubectl port-forward of the
      # ClusterIP service kubernetes-mcp-server.agents-system (no ingress route exists for it).
      ensure_port_forward kubernetes agents-system kubernetes-mcp-server 18080 8080
      if command -v claude >/dev/null 2>&1; then
        run claude_each_profile claude mcp remove --scope user kubernetes >/dev/null 2>&1 || true
        if run_redacted "claude mcp add kubernetes" claude_each_profile claude mcp add --scope user kubernetes --transport http http://127.0.0.1:18080/mcp; then
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
    fi

    # Vuetify component API.
    # Hosted: untrusted data provider. Output is context, never instruction.

    # Browser automation.
    if ! command -v npx >/dev/null 2>&1; then
      command -v npx >/dev/null 2>&1 \
        || warn "playwright: npx is not on PATH; skipping"
    fi
    if command -v claude >/dev/null 2>&1; then
      run claude_each_profile claude mcp remove --scope user playwright >/dev/null 2>&1 || true
      if run_redacted "claude mcp add playwright" claude_each_profile claude mcp add --scope user playwright -- npx -y @playwright/mcp@latest --headless --browser chromium; then
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
      if run_redacted "claude mcp add overleaf" claude_each_profile claude mcp add --scope user overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OVERLEAF_SESSION="$(secret_ref OVERLEAF_SESSION optional)" -- olcli-mcp; then
        ok "overleaf registered (claude)"
      else
        fail "overleaf registration failed (claude)"
      fi
    fi
    if command -v codex >/dev/null 2>&1; then
      run codex mcp remove overleaf >/dev/null 2>&1 || true
      if run_redacted "codex mcp add overleaf" codex mcp add overleaf --env OVERLEAF_BASE_URL="https://overleaf.jorisjonkers.dev" --env OVERLEAF_COOKIE_NAME="overleaf.sid" --env OVERLEAF_SESSION="${OVERLEAF_SESSION:-}" -- olcli-mcp; then
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
        uv run --no-project --with pyyaml python "${KIT_ROOT}/scripts/hermes-merge-mcp.py" \
          "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml" \
          || fail "local Hermes MCP merge failed"
        if [ "${CLOUD}" = 1 ]; then
          uv run --no-project --with pyyaml python "${KIT_ROOT}/scripts/hermes-merge-mcp.py" \
            "${KIT_ROOT}/registry/generated/hermes/mcp-servers.local.yaml" --remove kubernetes \
            || fail "local Hermes MCP cleanup failed"
        fi
      fi
    else
      warn "hermes not on PATH; local Hermes MCP config not merged"
    fi
    # Verify the VALUE, not the exit codes: ask Claude what it
    # actually has. Check expected servers are present, and report
    # any unexpected ones (but leave plugin-provided and hand-added).
    if [ "${CHECK_ONLY}" != 1 ]; then
     # Once per profile: each one answers for its own .claude.json.
     for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do
      registered=$(claude_in_profile "${profile_dir}" claude mcp list 2>/dev/null || true)
      # Check all expected servers are registered
      expected_servers=(memory-api memory-mcp playwright drawio overleaf)
      if [ "${CLOUD}" != 1 ]; then
        expected_servers+=(kubernetes)
      fi
      for want in "${expected_servers[@]}"; do
        case "${registered}" in
          *"${want}"*) ;;
          *) warn "MCP server ${want} is not registered in ${profile_dir}" ;;
        esac
      done
      # Report unknown servers (but ignore plugin-provided and hand-added ones)
      # One name per line, taken from the start of the line up to the first
      # colon. NOT a lookahead: BSD grep has no PCRE, so `(?=:)` is a
      # "repetition-operator operand invalid" error and the whole check
      # silently inspected nothing on macOS.
      echo "${registered}" | sed -n 's/^\([a-zA-Z0-9_:-]*\):[[:space:]].* - .*/\1/p' | sort -u | while read -r found; do
        case "${found}" in
          memory-api) ;;
          memory-mcp) ;;
          kubernetes) ;;
          vuetify) ;;
          playwright) ;;
          drawio) ;;
          overleaf) ;;
          plugin:*|idea|rubymine) ;;
          *) warn "unknown MCP server ${found} in ${profile_dir} -- hand-added or from a removed registry entry?" ;;
        esac
      done
     done
    fi
  else
    log "MCP registration skipped (--no-mcp)"
  fi
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

# Cross-model orchestrator (Claude + Codex) for large,
# decomposable work: triage, cross-model planning, grill, a design
# checkpoint, guarded fan-out, a review council, and resumable
# status/tail.
src="${KIT_ROOT}/skills/council"
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
fi

# Consult the current memory platform before designing or changing
# behavior that may depend on prior captures, and capture durable
# lessons near task completion.
src="${KIT_ROOT}/skills/kb-first"
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
fi

# Token-budget discipline: progressive disclosure, bounded recall,
# narrow MCP profiles, and prompt-cache-friendly instruction
# ordering.
src="${KIT_ROOT}/skills/token-economy"
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
fi

# Checklist for configuring a Claude Code or Codex session: config
# layers, MCP servers and profiles, memory-file hygiene, and
# Claude/Codex parity.
src="${KIT_ROOT}/skills/agent-session-bootstrap"
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
fi

# -----------------------------------------------------------------
# 8. Vendored third-party skills.
#
# Mirrors Hermes' sync-skills init container: clone the pinned ref,
# verify the commit sha (a mismatch is a supply-chain event, so it
# fails loudly rather than installing something else), then install
# every SKILL.md the source publishes into both agent homes.
# -----------------------------------------------------------------
log "vendored skills"

# No plugin wraps this one on the workstation, unlike caveman,
# kubernetes-skill, drawio and mattpocock-skills. 14 skills
# reaching only Hermes was exactly the gap agent-kit#40 closed.
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
        for vendor_home in "${CLAUDE_HOME}" "${CODEX_HOME}"; do
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
# 9. Retired hooks: actually purge them (see purge_retired_hooks above),
# not just report them. A machine set up by the retired install.sh /
# install-agents.sh is exactly the one that still has these.
# -----------------------------------------------------------------
purge_retired_hooks

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

# Report MCP servers skipped due to missing credentials
if [ "${#skipped_mcp_servers[@]}" -gt 0 ]; then
  log "MCP servers not registered (missing credentials):"
  for entry in "${skipped_mcp_servers[@]}"; do
    log "  - ${entry}"
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

# Report language servers with missing binaries
if [ "${#missing_lsp_binaries[@]}" -gt 0 ]; then
  log "Language servers with missing binaries:"
  for entry in "${missing_lsp_binaries[@]}"; do
    log "  - ${entry}"
  done
  log ""
fi

# Report plugins with unset required env vars
if [ "${#missing_plugin_env[@]}" -gt 0 ]; then
  log "Plugins with unset required env vars:"
  for entry in "${missing_plugin_env[@]}"; do
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
   && [ "${#disabled_plugins[@]}" = 0 ] && [ "${#plugin_drift[@]}" = 0 ] \
   && [ "${#missing_plugin_env[@]}" = 0 ]; then
  log "Setup complete: everything is ready"
else
  log "Setup summary: ${failures} failure(s), ${warnings} warning(s)"
fi

[ "${failures}" = 0 ]
