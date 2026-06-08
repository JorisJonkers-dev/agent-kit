#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --help|-h)
      printf 'Usage: %s [--json]\n' "$(basename "$0")"
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
plan_file=$(specify_plan_file "${branch}")
tasks_file=$(specify_tasks_file "${branch}")

[ -d "${feature_dir}" ] || specify_die "Feature directory not found: ${feature_dir}"
[ -f "${plan_file}" ] || specify_die "Implementation plan not found: ${plan_file}"

if [ ! -f "${tasks_file}" ]; then
  specify_copy_template "tasks-template.md" "${tasks_file}" "${branch}"
fi

if [ "${json}" = true ]; then
  specify_paths_json "${branch}"
else
  printf 'TASKS: %s\n' "${tasks_file}"
  printf 'IMPL_PLAN: %s\n' "${plan_file}"
  printf 'SPECS_DIR: %s\n' "${feature_dir}"
  printf 'BRANCH: %s\n' "${branch}"
fi
