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
spec_file=$(specify_spec_file "${branch}")
plan_file=$(specify_plan_file "${branch}")

mkdir -p "${feature_dir}" "${feature_dir}/contracts"
[ -f "${spec_file}" ] || specify_copy_template "spec-template.md" "${spec_file}" "${branch}"
[ -f "${plan_file}" ] || specify_copy_template "plan-template.md" "${plan_file}" "${branch}"

touch "${feature_dir}/research.md" "${feature_dir}/data-model.md" "${feature_dir}/quickstart.md"

if [ "${json}" = true ]; then
  specify_paths_json "${branch}"
else
  printf 'FEATURE_SPEC: %s\n' "${spec_file}"
  printf 'IMPL_PLAN: %s\n' "${plan_file}"
  printf 'SPECS_DIR: %s\n' "${feature_dir}"
  printf 'BRANCH: %s\n' "${branch}"
fi
