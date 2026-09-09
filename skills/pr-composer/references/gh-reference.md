# gh / glab reference for PR composition

Read this when the collector couldn't fetch something, or at execution time to get the flags right.

## Contents
- [Reading context](#reading-context)
- [Creating and updating](#creating-and-updating)
- [Reviewers, assignees, labels](#reviewers-assignees-labels)
- [Projects v2 (boards)](#projects-v2-boards)
- [Scopes and auth](#scopes-and-auth)
- [GitLab](#gitlab)

## Reading context

```bash
gh repo view --json nameWithOwner,visibility,isPrivate,defaultBranchRef,url
gh pr list --head "$(git branch --show-current)" --state all --json number,url,state,isDraft,body
gh pr view <n> --json number,title,body,reviewRequests,assignees,labels,projectItems
gh issue view <n> --json number,title,body,labels,assignees,url,projectItems
gh issue list --search "in:title <keyword>" --limit 10 --json number,title,url

# The 100-merge dump. `files` is the expensive field — drop it if the call is slow.
gh pr list --state merged --limit 100 --json \
  number,title,body,url,mergedAt,author,labels,assignees,reviews,reviewRequests,\
baseRefName,headRefName,files,additions,deletions,changedFiles
```

`--limit 100` costs several GraphQL pages. If it times out, retry at `--limit 50` and note the smaller sample in the profile rather than silently reporting rates from fewer PRs.

Which template GitHub itself would use: it prefers `.github/PULL_REQUEST_TEMPLATE.md`, then root, then `docs/`. A `.github/PULL_REQUEST_TEMPLATE/` **directory** means multiple named templates, and GitHub only applies one via the `?template=` query param — so the user has to choose. Never merge two templates together.

## Creating and updating

Always via `--body-file`. Bodies contain backticks, `$`, and newlines; inlining them into a shell string is how you end up with a PR body that has been partially evaluated by bash.

```bash
git push -u origin HEAD

gh pr create \
  --title "[MOVES-482] feat(scheduler): schedule against day-ahead prices" \
  --body-file /tmp/pr-ctx/body.md \
  --base main \
  --reviewer wvdberg,tvos \
  --assignee @me \
  --label backend,energy \
  --draft                     # omit for a ready PR

gh pr edit <n> --title "…" --body-file /tmp/pr-ctx/body.md
gh pr ready <n>               # flip draft → ready
```

`--reviewer` accepts users and `org/team` slugs. It fails the whole command if one handle is wrong, so if it errors, retry without reviewers and add them separately — the PR is the valuable part.

## Reviewers, assignees, labels

```bash
gh pr edit <n> --add-reviewer a,b --remove-reviewer c
gh pr edit <n> --add-assignee @me --remove-assignee x
gh pr edit <n> --add-label backend --remove-label wip
gh label list --limit 200 --json name,description
```

Note the asymmetry: `gh pr create` takes `--reviewer`, `gh pr edit` takes `--add-reviewer`. You cannot request review from yourself; filter `@me` out of the reviewer list before calling.

CODEOWNERS ownership for a specific path, when you want to be sure rather than pattern-matching the file yourself:

```bash
gh api "repos/{owner}/{repo}/codeowners/errors" --jq '.errors'   # validates the file
```

There's no clean API for "who owns path P", so match the CODEOWNERS globs against `files.txt` directly. Last matching rule wins — CODEOWNERS is last-match, not first-match, unlike `.gitignore` precedence intuitions.

## Projects v2 (boards)

```bash
gh project list --owner <org-or-user> --format json
gh project item-add <project-number> --owner <org> --url <pr-url>
gh project field-list <project-number> --owner <org> --format json
```

Setting a field value (e.g. Status → In Review) needs the item ID returned by `item-add`, plus field and option IDs:

```bash
gh project item-edit --id <item-id> --project-id <PVT_…> \
  --field-id <PVTSSF_…> --single-select-option-id <opt-id>
```

Two IDs that look interchangeable aren't: `--project-id` is the `PVT_` node ID from `project list --format json`, while `<project-number>` in `item-add` is the small integer from the board URL. Passing one where the other belongs produces an unhelpful "could not resolve" error.

If the linked issue is already on a board, putting the PR on the same board is usually what the user wants — check `projectItems` on the issue.

## Scopes and auth

`gh project` needs a scope the default login doesn't grant:

```bash
gh auth status
gh auth refresh -s project,read:project
```

This is interactive (it opens a browser and prints a device code), so don't run it unattended. When board addition fails on scopes, report the PR as created, name the board that wasn't set, and hand the user this command.

## GitLab

Templates live in `.gitlab/merge_request_templates/*.md`.

```bash
glab mr create --title "…" --description "$(cat body.md)" --target-branch main \
  --reviewer a,b --assignee me --label backend
glab mr update <n> --description "$(cat body.md)"
```

`glab` has no direct equivalent of GitHub Projects; GitLab boards are driven by labels, so board placement means setting the right label.
