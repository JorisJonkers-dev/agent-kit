---
name: open-pr
description: Open a pull request the way this repo requires — assignee, label, impersonal PR-body voice, no assistant attribution, and unescaped backticks in the heredoc.
---

# Opening a PR

Every PR is authored in the voice of the human driver, not an assistant
addressing them.

```bash
assignee="${AGENT_KIT_GH_ASSIGNEE:?set AGENT_KIT_GH_ASSIGNEE to the GitHub login that should own PRs}"
gh pr create --assignee "$assignee" --label <label> \
  --title "..." --body "$(cat <<'EOF'
...
EOF
)"
```

## Rules

- **`--assignee "$assignee"`** always, sourced from `AGENT_KIT_GH_ASSIGNEE`.
- **`--label`** the best single fit: `enhancement` (new capability),
  `bug` (fixes broken behaviour), `documentation` (docs/CLAUDE.md only),
  or `dependencies` + ecosystem (`docker`/`java`/`javascript`/
  `github_actions`) for version bumps. Stack labels only when the PR
  genuinely spans categories.
- **Voice:** impersonal, professional prose. No second-person ("you",
  "your"), no first-person-plural intent narration ("we now do X"), no
  hedging ("hopefully", "should be fine"). Lead with the observable
  behaviour or root cause, then the change: "X was doing Y; Z now does
  W because …". Past tense for the pre-PR state, present/future for the
  change. Rewrite dialogue into this voice — don't patch pronouns.
- **Never** add `Co-Authored-By`, "Generated with …", or the name of any
  AI assistant anywhere in the commit or PR body.
- **Backticks:** the heredoc delimiter is quoted (`<<'EOF'`), so write
  plain `` `foo` `` and ` ```lang ` — never backslash-escape them
  (that produces literal `\` in GitHub markdown).
- **Scope:** small, independently revertable PRs; stacking (B on A) is
  fine. Never bundle unrelated fixes. No "while we're here" refactors.

## After opening

Let CI go green, merge (squash), then `flux reconcile` the affected
kustomization if the change must land on the cluster promptly.
