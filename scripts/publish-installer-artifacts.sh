#!/usr/bin/env bash
# Attach setup-workstation.sh and the skills bundle to a GitHub release, and
# prove each landed by downloading it back rather than trusting `gh`'s exit
# code. Called by .github/workflows/publish-installer-artifacts.yml, one
# subcommand per step, so a failed attach still lets the rest run and report.
#
# Publishing to the public asset host (assets.jorisjonkers.dev) is a
# SEPARATE, in-cluster concern: a Flux-managed CronJob in fleet-infra pulls
# these release assets and uploads them to Garage over the ClusterIP S3 API.
# This script never talks to Garage or S3 -- it only touches the release.
#
# install.sh and install-agents.sh are retired by #40 and are deliberately
# never staged, attached, or included in the skills tarball.
#
# Config comes from the environment, set once at job level:
#   TAG_NAME    release tag, e.g. v4.1.0                       (required)
#   REPO_ROOT   checkout root                                  (default: this script's repo)
#   STAGE_DIR   scratch dir, shared across steps                (default: mktemp)
#   GH_TOKEN    passed straight to `gh`                         (required for attach-*/verify-*)
#   GH_REPO     owner/repo, passed straight to `gh`              (optional; gh auto-detects otherwise)
#
# Usage:
#   publish-installer-artifacts.sh stage
#   publish-installer-artifacts.sh verify-retired-excluded
#   publish-installer-artifacts.sh attach-script
#   publish-installer-artifacts.sh attach-skills
#   publish-installer-artifacts.sh attach-checksum
#   publish-installer-artifacts.sh verify-script
#   publish-installer-artifacts.sh verify-skills
#   publish-installer-artifacts.sh verify-checksum

set -euo pipefail

SCRIPT_NAME=setup-workstation.sh
# The "skills" bundle carries everything setup-workstation.sh reads beside
# itself; a copy run on its own re-runs the one inside it. The asset keeps
# its name because the asset host syncs it by that name.
SKILLS_TARBALL=agent-kit-skills.tar.gz
KIT_PATHS=(
  skills
  registry/estate-tooling.yaml
  registry/generated/hermes/mcp-servers.local.yaml
  installer/setup-workstation.sh
  installer/port-forward-agent.sh
  installer/cloud-session.sh
  scripts/hermes-merge-mcp.py
)
SKILLS_CHECKSUM="${SKILLS_TARBALL}.sha256"
RETIRED_NAMES=(install.sh install-agents.sh)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# REPO_ROOT is the checkout holding the PAYLOAD (installer/, skills/), which
# in CI is a release tag and may not carry this tooling script's own sibling
# helper. HELPER always resolves next to this script instead, so staging a
# historical tag still finds it.
REPO_ROOT="${REPO_ROOT:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
STAGE_DIR="${STAGE_DIR:-$(mktemp -d)}"
FETCHED_DIR="${STAGE_DIR}/fetched"
HELPER="${SCRIPT_DIR}/verify_release_asset.py"

log() { printf 'publish-installer-artifacts: %s\n' "$*"; }
die() { printf 'publish-installer-artifacts: %s\n' "$*" >&2; exit 1; }

require_env() {
  local name="$1"
  [ -n "${!name:-}" ] || die "\$${name} is not set"
}

gh_upload() { # gh_upload <local-file>
  require_env TAG_NAME
  local local_file="$1"
  gh release upload "${TAG_NAME}" "${local_file}" --clobber
  log "attached $(basename "${local_file}") to release ${TAG_NAME}"
}

gh_download() { # gh_download <remote-name>; prints the local path it saved to
  require_env TAG_NAME
  local remote_name="$1"
  mkdir -p "${FETCHED_DIR}"
  rm -f "${FETCHED_DIR:?}/${remote_name}"
  gh release download "${TAG_NAME}" --pattern "${remote_name}" --dir "${FETCHED_DIR}" --clobber
  [ -f "${FETCHED_DIR}/${remote_name}" ] || die "gh release download produced no ${remote_name}"
  printf '%s' "${FETCHED_DIR}/${remote_name}"
}

cmd_stage() {
  require_env TAG_NAME
  local path
  for path in "${KIT_PATHS[@]}"; do
    [ -e "${REPO_ROOT}/${path}" ] || die "no ${REPO_ROOT}/${path}"
  done
  rm -rf "${STAGE_DIR:?}"
  mkdir -p "${STAGE_DIR}"

  python3 "${HELPER}" stamp-version "${TAG_NAME}" \
    < "${REPO_ROOT}/installer/${SCRIPT_NAME}" \
    > "${STAGE_DIR}/${SCRIPT_NAME}"
  head -1 "${STAGE_DIR}/${SCRIPT_NAME}" | grep -qx '#!/usr/bin/env bash' \
    || die "stamping broke the shebang"

  # Plain tar flags only (both GNU and BSD tar): CI runs GNU tar, but keeping
  # this portable is what let the tarball logic be exercised locally. Content
  # is what a re-run must reproduce, not tar's own metadata bytes -- each run
  # is checksummed and verified against itself, not against a prior run.
  tar -C "${REPO_ROOT}" -czf "${STAGE_DIR}/${SKILLS_TARBALL}" "${KIT_PATHS[@]}"

  (cd "${STAGE_DIR}" && sha256sum "${SKILLS_TARBALL}" > "${SKILLS_CHECKSUM}")
  log "staged ${SCRIPT_NAME}, ${SKILLS_TARBALL} and ${SKILLS_CHECKSUM} in ${STAGE_DIR}"
}

cmd_verify_retired_excluded() {
  local name hits
  for name in "${RETIRED_NAMES[@]}"; do
    hits="$(find "${REPO_ROOT}" -type f -name "${name}" -not -path '*/.git/*' 2>/dev/null)"
    [ -z "${hits}" ] || die "${name} exists in the checkout (${hits}); it must not ship"
  done
  for name in "${RETIRED_NAMES[@]}"; do
    if tar -tzf "${STAGE_DIR}/${SKILLS_TARBALL}" | grep -E "(^|/)${name}\$" > /dev/null; then
      die "the skills tarball contains ${name}; it must not ship"
    fi
  done
  log "confirmed install.sh and install-agents.sh are excluded"
}

cmd_attach_script()   { gh_upload "${STAGE_DIR}/${SCRIPT_NAME}"; }
cmd_attach_skills()   { gh_upload "${STAGE_DIR}/${SKILLS_TARBALL}"; }
cmd_attach_checksum() { gh_upload "${STAGE_DIR}/${SKILLS_CHECKSUM}"; }

cmd_verify_script() {
  require_env TAG_NAME
  local fetched
  fetched="$(gh_download "${SCRIPT_NAME}")"
  python3 "${HELPER}" check-version "${TAG_NAME}" < "${fetched}" \
    || die "the ${SCRIPT_NAME} release asset's version token does not match release ${TAG_NAME}"
  log "verified the ${SCRIPT_NAME} release asset carries ${TAG_NAME}"
}

cmd_verify_skills() {
  local fetched
  fetched="$(gh_download "${SKILLS_TARBALL}")"
  python3 "${HELPER}" check-checksum "${fetched}" "${STAGE_DIR}/${SKILLS_CHECKSUM}" \
    || die "the ${SKILLS_TARBALL} release asset does not match its checksum"
  log "verified the ${SKILLS_TARBALL} release asset matches its checksum"
}

cmd_verify_checksum() {
  local fetched
  fetched="$(gh_download "${SKILLS_CHECKSUM}")"
  diff -u "${STAGE_DIR}/${SKILLS_CHECKSUM}" "${fetched}" \
    || die "the ${SKILLS_CHECKSUM} release asset differs from what we attached"
  log "verified the ${SKILLS_CHECKSUM} release asset matches"
}

case "${1:-}" in
  stage) cmd_stage ;;
  verify-retired-excluded) cmd_verify_retired_excluded ;;
  attach-script) cmd_attach_script ;;
  attach-skills) cmd_attach_skills ;;
  attach-checksum) cmd_attach_checksum ;;
  verify-script) cmd_verify_script ;;
  verify-skills) cmd_verify_skills ;;
  verify-checksum) cmd_verify_checksum ;;
  *) die "usage: $0 {stage|verify-retired-excluded|attach-script|attach-skills|attach-checksum|verify-script|verify-skills|verify-checksum}" ;;
esac
