#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
paths_only=false
require_tasks=false
include_tasks=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --paths-only)
      paths_only=true
      ;;
    --require-tasks)
      require_tasks=true
      ;;
    --include-tasks)
      include_tasks=true
      ;;
    --help|-h)
      printf 'Usage: %s [--json] [--paths-only] [--require-tasks] [--include-tasks]\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
  esac
  shift
done

branch=$(specify_require_feature_branch)
feature_dir=$(specify_feature_dir "${branch}")
spec_file=$(specify_spec_file "${branch}")
plan_file=$(specify_plan_file "${branch}")
tasks_file=$(specify_tasks_file "${branch}")

if [ "${paths_only}" = true ]; then
  if [ "${json}" = true ]; then
    specify_paths_json "${branch}"
  else
    printf 'REPO_ROOT: %s\n' "${REPO_ROOT}"
    printf 'BRANCH: %s\n' "${branch}"
    printf 'FEATURE_DIR: %s\n' "${feature_dir}"
    printf 'FEATURE_SPEC: %s\n' "${spec_file}"
    printf 'IMPL_PLAN: %s\n' "${plan_file}"
    printf 'TASKS: %s\n' "${tasks_file}"
  fi
  exit 0
fi

[ -d "${feature_dir}" ] || specify_die "Feature directory not found: ${feature_dir}"
[ -f "${spec_file}" ] || specify_die "Feature spec not found: ${spec_file}"
[ -f "${plan_file}" ] || specify_die "Implementation plan not found: ${plan_file}"

if [ "${require_tasks}" = true ] && [ ! -f "${tasks_file}" ]; then
  specify_die "Tasks file not found: ${tasks_file}"
fi

docs=""
append_doc() {
  if [ -n "${docs}" ]; then
    docs="${docs},"
  fi
  docs="${docs}\"$1\""
}

[ -f "${feature_dir}/research.md" ] && append_doc "research.md"
[ -f "${feature_dir}/data-model.md" ] && append_doc "data-model.md"
[ -d "${feature_dir}/contracts" ] && append_doc "contracts/"
[ -f "${feature_dir}/quickstart.md" ] && append_doc "quickstart.md"
if [ "${include_tasks}" = true ] && [ -f "${tasks_file}" ]; then
  append_doc "tasks.md"
fi

if [ "${json}" = true ]; then
  printf '{"FEATURE_DIR":"%s","AVAILABLE_DOCS":[%s]}\n' "$(specify_json_escape "${feature_dir}")" "${docs}"
else
  printf 'FEATURE_DIR: %s\n' "${feature_dir}"
  printf 'AVAILABLE_DOCS:\n'
  printf '%s\n' "${docs}" | tr ',' '\n' | sed 's/^/  /; s/"//g'
fi
