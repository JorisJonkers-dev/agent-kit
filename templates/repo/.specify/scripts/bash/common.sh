#!/usr/bin/env bash

set -euo pipefail

if [ -z "${SPECIFY_SCRIPT_DIR:-}" ]; then
  SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
fi

specify_repo_root() {
  if command -v git >/dev/null 2>&1 && git rev-parse --show-toplevel >/dev/null 2>&1; then
    git rev-parse --show-toplevel
    return 0
  fi

  CDPATH= cd -- "${SPECIFY_SCRIPT_DIR}/../../.." && pwd -P
}

REPO_ROOT=$(specify_repo_root)
SPECIFY_DIR="${REPO_ROOT}/.specify"
SPECS_DIR="${REPO_ROOT}/specs"

specify_has_git() {
  command -v git >/dev/null 2>&1 && git -C "${REPO_ROOT}" rev-parse --is-inside-work-tree >/dev/null 2>&1
}

specify_current_branch() {
  if specify_has_git; then
    git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD 2>/dev/null
  else
    basename "$(pwd)"
  fi
}

specify_is_feature_branch() {
  case "$1" in
    [0-9][0-9][0-9]-*) return 0 ;;
    *) return 1 ;;
  esac
}

specify_feature_dir() {
  printf '%s/specs/%s\n' "${REPO_ROOT}" "$1"
}

specify_spec_file() {
  printf '%s/specs/%s/spec.md\n' "${REPO_ROOT}" "$1"
}

specify_plan_file() {
  printf '%s/specs/%s/plan.md\n' "${REPO_ROOT}" "$1"
}

specify_tasks_file() {
  printf '%s/specs/%s/tasks.md\n' "${REPO_ROOT}" "$1"
}

specify_template_file() {
  printf '%s/templates/%s\n' "${SPECIFY_DIR}" "$1"
}

specify_json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

specify_error() {
  printf 'ERROR: %s\n' "$*" >&2
}

specify_die() {
  specify_error "$*"
  exit 1
}

specify_require_feature_branch() {
  current_branch=$(specify_current_branch)
  if [ "${current_branch}" = "main" ] || [ "${current_branch}" = "master" ]; then
    specify_die "Current branch '${current_branch}' is not a feature branch. Run create-new-feature.sh first."
  fi
  if ! specify_is_feature_branch "${current_branch}"; then
    specify_die "Current branch '${current_branch}' must start with a three-digit feature number, e.g. 001-my-feature."
  fi
  printf '%s\n' "${current_branch}"
}

specify_copy_template() {
  template_name=$1
  destination=$2
  feature_name=$3
  source_file=$(specify_template_file "${template_name}")

  [ -f "${source_file}" ] || specify_die "Template not found: ${source_file}"

  today=$(date +%F)
  sed \
    -e "s/{{FEATURE_NAME}}/${feature_name}/g" \
    -e "s/{{feature_name}}/${feature_name}/g" \
    -e "s/{{DATE}}/${today}/g" \
    "${source_file}" > "${destination}"
}

specify_paths_json() {
  branch=$1
  feature_dir=$(specify_feature_dir "${branch}")
  spec_file=$(specify_spec_file "${branch}")
  plan_file=$(specify_plan_file "${branch}")
  tasks_file=$(specify_tasks_file "${branch}")
  has_git=false
  if specify_has_git; then
    has_git=true
  fi

  printf '{"REPO_ROOT":"%s","BRANCH":"%s","HAS_GIT":"%s","FEATURE_DIR":"%s","FEATURE_SPEC":"%s","IMPL_PLAN":"%s","TASKS":"%s"}\n' \
    "$(specify_json_escape "${REPO_ROOT}")" \
    "$(specify_json_escape "${branch}")" \
    "${has_git}" \
    "$(specify_json_escape "${feature_dir}")" \
    "$(specify_json_escape "${spec_file}")" \
    "$(specify_json_escape "${plan_file}")" \
    "$(specify_json_escape "${tasks_file}")"
}
