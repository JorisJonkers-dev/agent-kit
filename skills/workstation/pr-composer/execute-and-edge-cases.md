### 6. Loop

Apply commands and re-render only what changed plus the footer, so the terminal doesn't fill with repeated full previews.

Two exceptions where the full card comes back:

- **Any edit that touches the body.** Rewrite `body.md`, re-run `pr-composer check-body`, then render the full body again. Reporting the write as a diff of the file is not a preview — the user cannot approve a PR from a list of removed lines.
- **Before asking to create**, after a batch of edits. When the user hands you several changes at once, apply them all, then show one complete card. Don't narrate each fix as you go and then ask for confirmation on a card nobody has seen in its final state.

Skipping is real: a skipped section is omitted from the body entirely, not filled with "N/A" — unless the style profile shows this repo does write "N/A" under unused headings, in which case match the repo.

If placeholders remain when the user says `create`, name them and ask once:

> `TESTING` still has an unfilled placeholder. Create anyway, or fill it first?

Accept "create anyway" and proceed. Don't ask twice, and don't quietly delete the placeholder — leaving `«…»` visible in the PR body is more honest than inventing test evidence, and reviewers read it as a prompt.

### 7. Execute, in order

Only after the gate opens. Do these one at a time and report each, because a partial failure mid-sequence is confusing otherwise:

```bash
git push -u origin HEAD                         # if unpushed
gh pr create --title "…" --body-file /tmp/pr-ctx/body.md \
  --base main --reviewer a,b --assignee joris --label x --draft
gh project item-add 7 --owner nedap --url <pr-url>
```

For an existing PR use `gh pr edit`. Write the body to a file rather than inlining it — shell quoting will eventually eat a backtick or a `$`.

If project-board addition fails on scopes, that's expected the first time; report it with the fix (`gh auth refresh -s project`) rather than treating the whole PR as failed. The PR exists; say so, give the URL.

### 8. Report

The URL, then a one-line list of what was set, then any placeholder still sitting in the body. Nothing else — the user is going to click the link.

## Edge cases worth handling

- **Not a GitHub remote.** GitLab uses `.gitlab/merge_request_templates/*.md` and `glab mr create`. The workflow is identical; say you're switching tooling. For anything else, produce the body and title and tell the user to paste them.
- **`gh` missing or unauthenticated.** Don't silently degrade — say what's unavailable, then offer to produce the full body as text.
- **Enormous diff** (say >50 files). Summarize by directory rather than enumerating files, and prefer the commit messages over the diff for intent.
- **Branch has one trivial commit.** Match the repo's floor: if this repo's small PRs get two-line bodies, don't produce eight sections. Overwriting a repo's culture with thoroughness is still overwriting it.
- **Merged-PR history is thin** (<10 PRs, new repo). Say the profile is weak and lean on the template plus conventional defaults, rather than reporting a confident-sounding convention derived from four samples.
