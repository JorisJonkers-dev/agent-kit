---
name: open-pr
description: Open a personal-stack pull request using the repo's required assignee, label, impersonal body voice, no assistant attribution, and clean GitHub Markdown heredoc formatting.
---

# Open PR

Create PRs in the human driver's voice, not as an assistant addressing them.

```bash
gh pr create --assignee ExtraToast --label <label> \
  --title "..." --body "$(cat <<'EOF'
...
EOF
)"
```

Rules:

- Use `--assignee ExtraToast`.
- Pick the best label: `enhancement`, `bug`, `documentation`, or
  `dependencies` plus ecosystem labels for version bumps. Stack labels only
  when the PR genuinely spans categories.
- Use impersonal, professional prose. Avoid second person, assistant
  attribution, hedging, and first-person-plural intent narration.
- Never add `Co-Authored-By`, generated-with text, or assistant names to commits
  or PR bodies.
- With a quoted heredoc delimiter (`<<'EOF'`), write plain backticks. Do not
  backslash-escape them.
- Keep PRs small and independently revertable. No unrelated cleanup.

After opening, let CI pass, merge squash, then reconcile affected Flux
kustomizations only when cluster state needs the change promptly.
