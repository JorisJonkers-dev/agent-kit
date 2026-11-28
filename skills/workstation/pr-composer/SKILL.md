---
name: pr-composer
description: Compose a pull request that matches the repository's own conventions — discovers the repo PR template, mines the last 100 merged PRs for house style, pulls in linked issues and Slack threads, suggests reviewers/assignees/project boards, and renders a visual preview that must be explicitly confirmed before anything is pushed or created. Use this skill whenever the user is opening, drafting, describing, retitling, or updating a pull request or merge request, mentions a PR template, asks "what should I write in this PR", asks for help with a PR body, wants reviewers or a project board set on a PR, or is about to run `gh pr create` — even if they don't say the word "skill" or ask for a template explicitly.
allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/collect.sh *) Bash(pr-composer mine-prs *) Bash(pr-composer check-body *) Bash(git log *) Bash(git diff *) Bash(git status *) Bash(git rev-parse *) Bash(git branch *) Bash(gh repo view *) Bash(gh pr list *) Bash(gh pr view *) Bash(gh issue view *) Bash(gh issue list *) Bash(gh label list *) Bash(gh project list *)
---

# PR Composer

Compose a pull request that looks like it was written by someone who has read the last hundred PRs in this repo — because you will have. Then show it, let the user rearrange it with short commands, and only touch GitHub once they say so.

## Non-negotiable: the confirmation gate

Nothing that changes remote state happens before the user explicitly confirms the rendered preview. That means no `git push`, no `gh pr create`, no `gh pr edit`, no `gh pr ready`, no label/assignee/reviewer changes, no `gh project item-add`, no comments. Reading is always fine.

The gate opens only on an explicit affirmative for *this* preview — `ok`, `create`, `go`, `ship it`, `yes do it`. It does not open on:

- an earlier "yes" given before the preview existed or before the last edit
- "looks good" as a reaction mid-editing (that's feedback, keep the loop open — ask "create it?")
- silence, or a new unrelated instruction
- your own judgement that the draft is obviously fine

The read-only commands this skill needs are pre-approved in `allowed-tools`; the mutating ones deliberately are not, so `git push`, `gh pr create`, `gh pr edit`, and `gh project item-add` still raise Claude Code's own permission prompt. That prompt is a second lock behind this gate, not a replacement for it — don't treat approving it as the user confirming the preview.

If a field changed since the last confirmation, the gate is closed again. Re-render, re-ask. Users tolerate one extra keystroke; they do not tolerate a PR appearing on a repo before they were ready.

## Workflow

### 1. Orient (read-only)

```bash
git rev-parse --abbrev-ref HEAD && git remote -v
gh repo view --json nameWithOwner,visibility,defaultBranchRef,isPrivate
gh pr status --json number,title,url,isDraft 2>/dev/null
```

Establish: is there already a PR for this branch? If yes, this is an *update* — load the existing body and treat the draft as an edit of it rather than a fresh write. Is the repo public? If so, note it; it changes what you're allowed to paste in from Slack (see step 3).

If the branch has unpushed commits, note it. Pushing is part of the gated action, not a preliminary.

### 2. Gather context

Run the collector, which handles template discovery, commit/diff summary, CODEOWNERS, reviewer history, and the 100-PR dump in one pass:

```bash
bash ${CLAUDE_SKILL_DIR}/scripts/collect.sh /tmp/pr-ctx
```

Then mine the history dump into a style profile rather than reading 100 PR bodies yourself — it's cheaper and the numbers are more honest than an impression:

```bash
pr-composer mine-prs /tmp/pr-ctx/merged_prs.json --out /tmp/pr-ctx/style.json --md
```

Read `/tmp/pr-ctx/style.md` (short) and the template file. `${CLAUDE_SKILL_DIR}/references/gh-reference.md` has the exact commands for anything the collector couldn't get, plus the project-board GraphQL and scope-refresh incantations.

**Template handling.** The template is a contract, not a suggestion: keep its headings, their order, and their exact wording. Keep HTML comments that are instructions to the author only if the repo's merged PRs also keep them — the style profile reports this as `comments_retained_rate`. If `.github/PULL_REQUEST_TEMPLATE/` holds several templates, ask which one; don't guess. If there is no template at all, synthesize one from the section headings that appear most often in merged PRs, and say that's what you did.

**Style profile.** Use it for concrete decisions, not vibes: title prefix convention (conventional-commit, ticket key, bare), whether titles are imperative, typical body length, which sections actually get filled in versus left empty, the issue-linking keyword this repo uses, and the top reviewers. If the repo writes `[MOVES-482] fix: …`, your title does too.

### 3. Ask for issues and Slack threads

**This step blocks the first preview.** A preview whose link fields read `«fill in or remove»` is a preview that wasted the user's turn: they now have to answer a question you could have asked before drafting, and then re-read the whole card. If the template has a links section, its content is an input to the draft, not a placeholder to be filled in later.

Ask once, batched, with what you already found pre-filled so the user is confirming rather than typing:

> Found in your branch: `#482` (referenced in commit `a3f1c2`), `#490` (branch name). Anything else — issues, or Slack threads I should pull the reasoning from?

Resolve issues read-only and use their content:

```bash
gh issue view 482 --json number,title,body,labels,assignees,url
```

Accept "none" as a real answer and move on — plenty of PRs have no issue and no thread. When the user says none, drop the link lines rather than leaving empty bullets, unless the template's own structure requires the heading to stay.

For Slack: check whether a Slack tool is connected. If one is, fetch the threads and use them. If not, ask for permalinks plus a paste of the relevant messages — do not pretend to have read a thread you only have a URL for.

**Two rules about Slack content.** Never quote a person's Slack message verbatim into a PR body; synthesize the decision ("we chose the ground-source path because the ROI window was inside 8 years") and cite the thread by permalink. And if the repo is public while the Slack workspace is internal, say so and ask before including anything from it. A leaked internal discussion is not recoverable.

## Read next

The rest of this skill is split into part files in this folder. Open the one you need; the rules in them are part of this skill.

- [`draft-and-preview.md`](draft-and-preview.md): 4. Draft; 5. Render the preview
- [`execute-and-edge-cases.md`](execute-and-edge-cases.md): 6. Loop; 7. Execute, in order; 8. Report; Edge cases worth handling
