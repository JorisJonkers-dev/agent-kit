#!/usr/bin/env bash
# Publish setup-workstation.sh and the skills bundle to the public asset host,
# and prove each landed by fetching it back rather than trusting curl's exit
# code. Called by .github/workflows/publish-installer-artifacts.yml, one
# subcommand per step, so a failed upload still lets the others run and
# report instead of hiding behind an early exit.
#
# install.sh and install-agents.sh are retired by #40 and are deliberately
# never staged or uploaded here.
#
# Config comes from the environment, set once at job level:
#   TAG_NAME                  release tag, e.g. v4.1.0            (required)
#   REPO_ROOT                 checkout root                       (default: this script's repo)
#   STAGE_DIR                 scratch dir, shared across steps     (default: mktemp)
#   BUCKET                    bucket name, also the public host    (default: assets.jorisjonkers.dev)
#   PUBLIC_URL                anonymous-read website endpoint      (default: https://$BUCKET)
#   S3_ENDPOINT                signed S3 API endpoint for writes    (required for upload-*)
#   GARAGE_ACCESS_KEY_ID       (required for upload-*)
#   GARAGE_SECRET_ACCESS_KEY   (required for upload-*)
#
# Usage:
#   publish-installer-artifacts.sh stage
#   publish-installer-artifacts.sh upload-script
#   publish-installer-artifacts.sh upload-skills
#   publish-installer-artifacts.sh upload-checksum
#   publish-installer-artifacts.sh verify-script
#   publish-installer-artifacts.sh verify-skills
#   publish-installer-artifacts.sh verify-checksum
#   publish-installer-artifacts.sh verify-retired-absent

set -euo pipefail

SCRIPT_NAME=setup-workstation.sh
SKILLS_TARBALL=agent-kit-skills.tar.gz
SKILLS_CHECKSUM="${SKILLS_TARBALL}.sha256"
SIGV4_SERVICE="aws:amz:garage:s3"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# REPO_ROOT is the checkout holding the PAYLOAD (installer/, skills/), which
# in CI is a release tag and may not carry this tooling script's own sibling
# helper. HELPER always resolves next to this script instead, so staging a
# historical tag still finds it.
REPO_ROOT="${REPO_ROOT:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
STAGE_DIR="${STAGE_DIR:-$(mktemp -d)}"
BUCKET="${BUCKET:-assets.jorisjonkers.dev}"
PUBLIC_URL="${PUBLIC_URL:-https://${BUCKET}}"
FETCHED_DIR="${STAGE_DIR}/fetched"
HELPER="${SCRIPT_DIR}/verify_release_asset.py"

log() { printf 'publish-installer-artifacts: %s\n' "$*"; }
die() { printf 'publish-installer-artifacts: %s\n' "$*" >&2; exit 1; }

require_env() {
  local name="$1"
  [ -n "${!name:-}" ] || die "\$${name} is not set"
}

content_type_for() {
  case "$1" in
    "${SCRIPT_NAME}") printf 'text/x-shellscript; charset=utf-8' ;;
    "${SKILLS_TARBALL}") printf 'application/gzip' ;;
    *) printf 'text/plain; charset=utf-8' ;;
  esac
}

put_object() { # put_object <local-file> <remote-key>
  local local_file="$1" remote_key="$2" content_type status
  require_env S3_ENDPOINT
  require_env GARAGE_ACCESS_KEY_ID
  require_env GARAGE_SECRET_ACCESS_KEY
  content_type="$(content_type_for "${remote_key}")"
  status="$(curl -sS --aws-sigv4 "${SIGV4_SERVICE}" \
    --user "${GARAGE_ACCESS_KEY_ID}:${GARAGE_SECRET_ACCESS_KEY}" \
    -X PUT --upload-file "${local_file}" \
    -H "Content-Type: ${content_type}" \
    -o /dev/null -w '%{http_code}' \
    "${S3_ENDPOINT%/}/${BUCKET}/${remote_key}")"
  [ "${status}" = "200" ] || die "PUT ${remote_key} returned ${status}"
  log "uploaded ${remote_key} (${content_type})"
}

fetch_public() { # fetch_public <remote-key>; prints the local path it saved to
  local remote_key="$1" out_file status
  mkdir -p "${FETCHED_DIR}"
  out_file="${FETCHED_DIR}/${remote_key}"
  # No Authorization header: the whole point of the website endpoint is an
  # anonymous GET, and sending a credential here would hide a bucket
  # misconfigured for anonymous read behind a signature that papers over it.
  status="$(curl -sS -o "${out_file}" -w '%{http_code}' "${PUBLIC_URL}/${remote_key}")"
  [ "${status}" = "200" ] || die "GET ${PUBLIC_URL}/${remote_key} returned ${status}"
  printf '%s' "${out_file}"
}

cmd_stage() {
  require_env TAG_NAME
  [ -f "${REPO_ROOT}/installer/${SCRIPT_NAME}" ] || die "no ${REPO_ROOT}/installer/${SCRIPT_NAME}"
  [ -d "${REPO_ROOT}/skills" ] || die "no ${REPO_ROOT}/skills directory"
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
  tar -C "${REPO_ROOT}" -czf "${STAGE_DIR}/${SKILLS_TARBALL}" skills

  (cd "${STAGE_DIR}" && sha256sum "${SKILLS_TARBALL}" > "${SKILLS_CHECKSUM}")
  log "staged ${SCRIPT_NAME}, ${SKILLS_TARBALL} and ${SKILLS_CHECKSUM} in ${STAGE_DIR}"
}

cmd_upload_script()   { put_object "${STAGE_DIR}/${SCRIPT_NAME}" "${SCRIPT_NAME}"; }
cmd_upload_skills()   { put_object "${STAGE_DIR}/${SKILLS_TARBALL}" "${SKILLS_TARBALL}"; }
cmd_upload_checksum() { put_object "${STAGE_DIR}/${SKILLS_CHECKSUM}" "${SKILLS_CHECKSUM}"; }

cmd_verify_script() {
  require_env TAG_NAME
  local fetched
  fetched="$(fetch_public "${SCRIPT_NAME}")"
  python3 "${HELPER}" check-version "${TAG_NAME}" < "${fetched}" \
    || die "published ${SCRIPT_NAME} version token does not match release ${TAG_NAME}"
  log "verified ${PUBLIC_URL}/${SCRIPT_NAME} carries ${TAG_NAME}"
}

cmd_verify_skills() {
  local fetched
  fetched="$(fetch_public "${SKILLS_TARBALL}")"
  python3 "${HELPER}" check-checksum "${fetched}" "${STAGE_DIR}/${SKILLS_CHECKSUM}" \
    || die "published ${SKILLS_TARBALL} does not match its checksum"
  log "verified ${PUBLIC_URL}/${SKILLS_TARBALL} matches its checksum"
}

cmd_verify_checksum() {
  local fetched
  fetched="$(fetch_public "${SKILLS_CHECKSUM}")"
  diff -u "${STAGE_DIR}/${SKILLS_CHECKSUM}" "${fetched}" \
    || die "published ${SKILLS_CHECKSUM} differs from what we uploaded"
  log "verified ${PUBLIC_URL}/${SKILLS_CHECKSUM} matches"
}

cmd_verify_retired_absent() {
  local name status
  for name in install.sh install-agents.sh; do
    status="$(curl -s -o /dev/null -w '%{http_code}' "${PUBLIC_URL}/${name}")"
    [ "${status}" = "404" ] || die "${name} is served (${status}); it must not be published"
  done
  log "confirmed install.sh and install-agents.sh are not published"
}

case "${1:-}" in
  stage) cmd_stage ;;
  upload-script) cmd_upload_script ;;
  upload-skills) cmd_upload_skills ;;
  upload-checksum) cmd_upload_checksum ;;
  verify-script) cmd_verify_script ;;
  verify-skills) cmd_verify_skills ;;
  verify-checksum) cmd_verify_checksum ;;
  verify-retired-absent) cmd_verify_retired_absent ;;
  *) die "usage: $0 {stage|upload-script|upload-skills|upload-checksum|verify-script|verify-skills|verify-checksum|verify-retired-absent}" ;;
esac
