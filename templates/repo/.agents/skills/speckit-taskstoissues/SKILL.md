---
name: speckit-taskstoissues
description: Create GitHub issues from the active Spec Kit tasks list.
---

# Speckit Tasks To Issues

Use this skill for the `$speckit-taskstoissues` phase. Support `--dry-run` by
printing the `gh issue create` commands without creating issues.

## Bootstrap

If `.specify` is absent, create `.specify/memory`, `.specify/templates`,
`.specify/scripts/bash`, and `specs`. Add a minimal
`.specify/scripts/bash/check-prerequisites.sh` only when absent. It must support
`--json --require-tasks --include-tasks` and return `FEATURE_DIR`.

## Workflow

1. Run `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks`.
   Stop if `FEATURE_DIR/tasks.md` is missing.
2. Verify `gh` is installed and authenticated unless running `--dry-run`.
3. Parse task lines from `tasks.md` that start with a Markdown task checkbox and
   contain a `T###` task id.
4. Create one issue per task with title
   `[SpecKit] <feature-dir-name>: <task-id> <task>`.
5. Before creating, check all repository issues by exact title; warn and skip
   already-created issues.
6. Always include `--assignee ExtraToast`.
7. Pick exactly one best-fit existing repo label from
   `enhancement`, `bug`, or `documentation`. Never invent labels; if the chosen
   label does not exist, warn and omit `--label`.
8. Use this inline helper shape; do not add a script file:

```bash
dry_run=false
case " $ARGUMENTS " in *" --dry-run "*) dry_run=true ;; esac

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
    printf 'gh issue create --title %q --body %q --assignee ExtraToast' "$title" "$body"
    [ "${#label_args[@]}" -eq 0 ] || printf ' --label %q' "$label"
    printf '\n'
  else
    gh issue create --title "$title" --body "$body" --assignee ExtraToast "${label_args[@]}"
  fi
done < <(grep -E '^- \[[ xX]\].*T[0-9]{3,}' "$tasks_file")
```

9. Report created, skipped, and dry-run counts.
