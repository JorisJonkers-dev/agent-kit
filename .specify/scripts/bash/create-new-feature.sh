#!/usr/bin/env bash

set -euo pipefail

SPECIFY_SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
. "${SPECIFY_SCRIPT_DIR}/common.sh"

json=false
feature_number=""
description=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --json)
      json=true
      ;;
    --number)
      shift
      [ "$#" -gt 0 ] || specify_die "--number requires a value"
      feature_number=$1
      ;;
    --help|-h)
      printf 'Usage: %s [--json] [--number N] <feature description>\n' "$(basename "$0")"
      exit 0
      ;;
    --*)
      specify_die "Unknown option: $1"
      ;;
    *)
      if [ -z "${description}" ]; then
        description=$1
      else
        description="${description} $1"
      fi
      ;;
  esac
  shift
done

[ -n "${description}" ] || specify_die "Feature description is required"

slug=$(printf '%s\n' "${description}" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/ /g' | awk '
{
  count = 0
  for (i = 1; i <= NF; i++) {
    word = $i
    if (word == "the" || word == "and" || word == "for" || word == "with" || word == "from" || word == "that" || word == "this" || word == "into" || word == "onto") {
      continue
    }
    if (length(word) < 2) {
      continue
    }
    words[++count] = word
    if (count == 5) {
      break
    }
  }
  if (count == 0) {
    print "feature"
  } else {
    for (i = 1; i <= count; i++) {
      printf "%s%s", (i == 1 ? "" : "-"), words[i]
    }
    printf "\n"
  }
}')

highest=0
if specify_has_git; then
  branches=$(git -C "${REPO_ROOT}" branch --all --format='%(refname:short)' 2>/dev/null || true)
  for ref in ${branches}; do
    ref=${ref#origin/}
    number=${ref%%-*}
    case "${number}" in
      [0-9][0-9][0-9])
        if [ "${number}" -gt "${highest}" ]; then
          highest=${number}
        fi
        ;;
    esac
  done
fi

if [ -d "${SPECS_DIR}" ]; then
  for path in "${SPECS_DIR}"/[0-9][0-9][0-9]-*; do
    [ -d "${path}" ] || continue
    name=$(basename "${path}")
    number=${name%%-*}
    case "${number}" in
      [0-9][0-9][0-9])
        if [ "${number}" -gt "${highest}" ]; then
          highest=${number}
        fi
        ;;
    esac
  done
fi

if [ -n "${feature_number}" ]; then
  case "${feature_number}" in
    *[!0-9]*) specify_die "--number must be numeric" ;;
  esac
  number=$(printf '%03d' "${feature_number}")
else
  number=$(printf '%03d' $((highest + 1)))
fi

branch_name="${number}-${slug}"
feature_dir=$(specify_feature_dir "${branch_name}")
spec_file=$(specify_spec_file "${branch_name}")

if specify_has_git; then
  if ! git -C "${REPO_ROOT}" rev-parse --verify --quiet "${branch_name}" >/dev/null; then
    git -C "${REPO_ROOT}" checkout -b "${branch_name}" >/dev/null 2>&1
  else
    git -C "${REPO_ROOT}" checkout "${branch_name}" >/dev/null 2>&1
  fi
fi

mkdir -p "${feature_dir}"
if [ ! -f "${spec_file}" ]; then
  specify_copy_template "spec-template.md" "${spec_file}" "${branch_name}"
fi

if [ "${json}" = true ]; then
  printf '{"BRANCH_NAME":"%s","SPEC_FILE":"%s","FEATURE_DIR":"%s","FEATURE_NUMBER":"%s"}\n' \
    "$(specify_json_escape "${branch_name}")" \
    "$(specify_json_escape "${spec_file}")" \
    "$(specify_json_escape "${feature_dir}")" \
    "$(specify_json_escape "${number}")"
else
  printf 'BRANCH_NAME: %s\n' "${branch_name}"
  printf 'SPEC_FILE: %s\n' "${spec_file}"
fi
