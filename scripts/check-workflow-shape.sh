#!/usr/bin/env bash
# Runs `workflow-lint` over every Gold Band workflow template, once per rule group, and enforces
# each group's known-failures list (JorisJonkers-dev/agent-kit#781, #793).
#
#   scripts/check-workflow-shape.sh WORKFLOW_LINT_BINARY [WORKFLOWS_DIR]
#
# Groups: `--shape` against WORKFLOWS_DIR/known-shape-failures.txt (one file name per line) and
# `--critique` against WORKFLOWS_DIR/known-critique-failures.txt (`<file> #<ticket>` per line; the
# ticket that fixes it is required). The lists are separate. A template that fails a group and is
# listed for it is tolerated and reported. A template that fails and is not listed fails the run,
# and so does a listed template that now passes ("remove it from the list"), a critique line with
# no ticket, or a name that is no longer a template. A `*-work.json` template is linted as the work
# instance, every other one as the personal instance.
set -euo pipefail

lint=${1:?usage: check-workflow-shape.sh WORKFLOW_LINT_BINARY [WORKFLOWS_DIR]}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dir=${2:-$root/installer/gold-band/workflows}

failed=0
shopt -s nullglob
files=("$dir"/*.json)
[ "${#files[@]}" -gt 0 ] || { echo "workflow shape: no templates in $dir" >&2; exit 1; }

# check_group FLAG LIST_FILE LABEL REQUIRE_TICKET
check_group() {
    local flag=$1 known=$2 label=$3 ticket=$4
    local -A listed=()
    local line name path instance rc output
    while IFS= read -r line; do
        case $line in '' | '#'*) continue ;; esac
        name=${line%%[[:space:]]*}
        if [ "$ticket" = yes ] && ! [[ $line =~ [[:space:]]#[0-9]+ ]]; then
            echo "::error file=$known::$name has no ticket (#N) that fixes it"
            failed=1
        fi
        listed[$name]=1
    done <"$known"
    for path in "${files[@]}"; do
        name=$(basename "$path")
        instance=gold-band-personal
        case $name in *-work.json) instance=gold-band ;; esac
        rc=0
        output=$("$lint" "$flag" --instance "$instance" "$path" 2>&1) || rc=$?
        if [ "$rc" -eq 2 ]; then
            echo "::error file=$path::$output"
            failed=1
        elif [ "$rc" -eq 0 ]; then
            if [ -n "${listed[$name]:-}" ]; then
                echo "::error file=$path::$name now passes the $label rules; remove it from $(basename "$known")"
                failed=1
            else
                echo "ok    $name ($label)"
            fi
        elif [ -n "${listed[$name]:-}" ]; then
            echo "known $name fails the $label rules (tolerated, see $(basename "$known"))"
        else
            echo "$output" >&2
            echo "::error file=$path::$name fails the $label rules and is not in $(basename "$known")"
            failed=1
        fi
        unset "listed[$name]"
    done
    for name in "${!listed[@]}"; do
        echo "::error file=$known::$name is listed as a known $label failure but is not a template in $dir; remove it from the list"
        failed=1
    done
}

check_group --shape "$dir/known-shape-failures.txt" shape no
check_group --critique "$dir/known-critique-failures.txt" critique yes
exit "$failed"
