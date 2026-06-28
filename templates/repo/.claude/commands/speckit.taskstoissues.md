---
description: Create GitHub issues from the active Spec Kit tasks list.
---

## User Input

```text
$ARGUMENTS
```

Recognize `--dry-run`. In dry-run mode, print the `gh issue create` commands
that would run and do not create issues.

## Bootstrap

If `.specify` is absent, scaffold `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` if missing. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR` plus available
docs. Do not overwrite existing scripts or templates.

## Procedure

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`
   from the repo root. Stop if `FEATURE_DIR/tasks.md` is missing.
2. Verify `gh` is available and authenticated unless this is `--dry-run`.
3. Read `FEATURE_DIR/tasks.md` and create one issue per task line that starts
   with a Markdown task checkbox and contains a `T###` task id.
4. Use titles in this format: `[SpecKit] <feature-dir-name>: <task-id> <task>`.
5. Before creating each issue, check all repository issues by exact title. If an
   issue with the same title already exists, warn and skip it.
6. Assign every created issue with `--assignee "$assignee"` from
   `AGENT_KIT_GH_ASSIGNEE`.
7. Pick exactly one best-fit label from the existing repository labels:
   `bug`, `documentation`, or `enhancement`.
   - Use `bug` for tasks about bugs, fixes, regressions, failures, or defects.
   - Use `documentation` for docs, README, runbook, guide, or content tasks.
   - Otherwise use `enhancement`.
   - Never invent labels. If the chosen label does not exist in the repository,
     warn and omit `--label`.
8. Use this helper shape inline; do not add scripts outside this command:

```bash
dry_run=false
case " $ARGUMENTS " in *" --dry-run "*) dry_run=true ;; esac
assignee="${AGENT_KIT_GH_ASSIGNEE:?set AGENT_KIT_GH_ASSIGNEE to the GitHub login that should own generated issues}"

feature_dir="<FEATURE_DIR from check-prerequisites>"
tasks_file="$feature_dir/tasks.md"
feature_name="$(basename "$feature_dir")"

label_exists() {
  gh label list --limit 1000 --json name --jq '.[].name' | grep -Fxq "$1"
}

issue_exists() {
  gh issue list --state all --search "$1 in:title" --json title --jq '.[].title' | grep -Fxq "$1"
}

while IFS= read -r task_line; do
  task_id="$(printf '%s\n' "$task_line" | grep -Eo 'T[0-9]{3,}' | head -n 1)"
  task_text="$(printf '%s\n' "$task_line" |
    sed -E 's/^- \[[ xX]\][[:space:]]*//; s/\[P\][[:space:]]*//g; s/T[0-9]{3,}[[:space:]]*//; s/^[[:space:]]+//')"
  [ -n "$task_id" ] || continue
  [ -n "$task_text" ] || continue

  title="[SpecKit] ${feature_name}: ${task_id} ${task_text}"
  body="$(cat <<EOF
## Task

${task_line}

## Source

- Feature: ${feature_dir}
- Tasks: ${tasks_file}
EOF
)"

  lower="$(printf '%s\n' "$task_text" | tr '[:upper:]' '[:lower:]')"
  label="enhancement"
  case "$lower" in
    *bug*|*fix*|*regression*|*failure*|*defect*) label="bug" ;;
    *doc*|*docs*|*documentation*|*readme*|*runbook*|*guide*) label="documentation" ;;
  esac

  label_args=()
  if label_exists "$label"; then
    label_args=(--label "$label")
  else
    printf 'WARN: label %s does not exist; omitting --label for %s\n' "$label" "$title" >&2
  fi

  if issue_exists "$title"; then
    printf 'WARN: issue already exists, skipping: %s\n' "$title" >&2
    continue
  fi

  if [ "$dry_run" = true ]; then
    printf 'gh issue create --title %q --body %q --assignee %q' "$title" "$body" "$assignee"
    [ "${#label_args[@]}" -eq 0 ] || printf ' --label %q' "$label"
    printf '\n'
  else
    gh issue create --title "$title" --body "$body" --assignee "$assignee" "${label_args[@]}"
  fi
done < <(grep -E '^- \[[ xX]\].*T[0-9]{3,}' "$tasks_file")
```

9. Report created, skipped, and dry-run counts.
