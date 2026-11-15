#!/usr/bin/env bash
# Runs `workflow-lint --shape` over every Gold Band workflow template and
# enforces the known-failures list (JorisJonkers-dev/agent-kit#781).
#
#   scripts/check-workflow-shape.sh WORKFLOW_LINT_BINARY [WORKFLOWS_DIR]
#
# A template that fails and is listed in WORKFLOWS_DIR/known-shape-failures.txt
# is tolerated and reported. A template that fails and is not listed fails the
# run, and so does a listed template that now passes ("remove it from the
# list") or names a file that no longer exists. A `*-work.json` template is
# linted as the work instance, every other one as the personal instance.
set -euo pipefail

lint=${1:?usage: check-workflow-shape.sh WORKFLOW_LINT_BINARY [WORKFLOWS_DIR]}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dir=${2:-$root/installer/gold-band/workflows}
known=$dir/known-shape-failures.txt

declare -A listed=()
while IFS= read -r line; do
    case $line in '' | '#'*) continue ;; esac
    listed[$line]=1
done <"$known"

failed=0
shopt -s nullglob
files=("$dir"/*.json)
[ "${#files[@]}" -gt 0 ] || { echo "workflow shape: no templates in $dir" >&2; exit 1; }
for path in "${files[@]}"; do
    name=$(basename "$path")
    instance=gold-band-personal
    case $name in *-work.json) instance=gold-band ;; esac
    rc=0
    output=$("$lint" --shape --instance "$instance" "$path" 2>&1) || rc=$?
    if [ "$rc" -eq 2 ]; then
        echo "::error file=$path::$output"
        failed=1
    elif [ "$rc" -eq 0 ]; then
        if [ -n "${listed[$name]:-}" ]; then
            echo "::error file=$path::$name now passes the shape rules; remove it from known-shape-failures.txt"
            failed=1
        else
            echo "ok    $name"
        fi
    elif [ -n "${listed[$name]:-}" ]; then
        echo "known $name fails the shape rules (tolerated, #781)"
    else
        echo "$output" >&2
        echo "::error file=$path::$name fails the shape rules and is not in known-shape-failures.txt"
        failed=1
    fi
    unset "listed[$name]"
done
for name in "${!listed[@]}"; do
    echo "::error file=$known::$name is listed as a known failure but is not a template in $dir; remove it from the list"
    failed=1
done
exit "$failed"
