---
name: pr-composer
description: Compose a pull request that matches the repository's own conventions — discovers the repo PR template, mines the last 100 merged PRs for house style, pulls in linked issues and Slack threads, suggests reviewers/assignees/project boards, and renders a visual preview that must be explicitly confirmed before anything is pushed or created. Use this skill whenever the user is opening, drafting, describing, retitling, or updating a pull request or merge request, mentions a PR template, asks "what should I write in this PR", asks for help with a PR body, wants reviewers or a project board set on a PR, or is about to run `gh pr create` — even if they don't say the word "skill" or ask for a template explicitly.
allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/collect.sh *) Bash(python3 ${CLAUDE_SKILL_DIR}/scripts/mine_prs.py *) Bash(python3 ${CLAUDE_SKILL_DIR}/scripts/check_body.py *) Bash(git log *) Bash(git diff *) Bash(git status *) Bash(git rev-parse *) Bash(git branch *) Bash(gh repo view *) Bash(gh pr list *) Bash(gh pr view *) Bash(gh issue view *) Bash(gh issue list *) Bash(gh label list *) Bash(gh project list *)
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
python3 ${CLAUDE_SKILL_DIR}/scripts/mine_prs.py /tmp/pr-ctx/merged_prs.json --out /tmp/pr-ctx/style.json --md
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

### 4. Draft

Fill the template. Where a fact is not in evidence — test results you haven't seen, a benchmark number, a ticket ID, a screenshot, a migration rollback plan — insert a placeholder rather than a plausible sentence:

```
«benchmark: p95 before → after»
```

Placeholders are a feature. Count them and surface the count in the preview. Invented specifics are the one failure mode that makes this skill worse than writing the PR by hand, because they read as verified.

**Fill the template; don't improve it.** Four rules, all of them checked mechanically by `check_body.py` in the next step:

1. No section that isn't in the template. If you want a `Verification` or `Notes` heading and the template doesn't have one, the answer is no — put the content under whichever existing heading covers it, or ask.
2. No reordering. Link blocks especially: if the template opens with links, they open the body, however much better they'd read at the bottom.
3. Heading levels verbatim. `##` stays `##`.
4. Checklists complete. Every item, wording unchanged, ticked only where you have evidence it's true. Checklist items are review gates; dropping one removes a gate.

Derive these alongside the body:

- **Title** — follow the repo's convention exactly, from the style profile.
- **Reviewers** — intersect CODEOWNERS for the touched paths with the repo's frequent reviewers, and prefer people who have reviewed these specific files before (`reviewer_by_path` in the profile). Cap at what the repo actually uses on average; don't request six people because you can.
- **Assignee** — the author, unless the repo's history says otherwise.
- **Labels** — only labels that exist (`gh label list`) and that co-occur with this kind of change in history. Check `meta.required_label_families` in the style profile first: a label family carried by nearly every merged PR (a `version/*` set, a release marker, a type marker) is effectively mandatory, and shipping without one is a finding, not an omission. If the family is mandatory but the right value is ambiguous, that's the one thing worth flagging.
- **Project board(s)** — from `gh project list`; propose the one the linked issue already sits on, if any.
- **Draft status** — draft if the branch has WIP/fixup commits or failing local checks.

### 5. Render the preview

**The preview shows the body verbatim.** Not a summary, not uppercased section names, not fields lifted out into a metadata table. Every `#` and `##` exactly as it will appear on GitHub, every checklist item, in template order. The user is approving the artifact, so a preview that differs from the artifact defeats the entire gate — and it hides exactly the errors this step exists to catch, because an invented section or a dropped checklist item looks fine once it's been paraphrased into a tidy field.

First validate the body against the template:

```bash
python3 ${CLAUDE_SKILL_DIR}/scripts/check_body.py \
  --template /tmp/pr-ctx/template.md --body /tmp/pr-ctx/body.md \
  --skipped "Screenshots"
```

Exit 1 means fix the body and re-run. Don't render a preview over a failing check — invented sections, changed heading levels, reordered link blocks, and truncated checklists all pass a visual skim.

Numbers go in a left gutter beside heading lines only, so the markdown itself stays untouched. Metadata continues the numbering below.

```
── PR PREVIEW ─ nedap/moves (private) ─ feature/tooltip-fix → master ──
new PR · draft · 7 commits, 14 files (+210 −64) · 46 behind master

TITLE
  0 │ [MOVES-482] fix: tooltip rendering and drag menu targeting

BODY (verbatim)
  1 │ ## Links
    │ - GitHub issue: #482
    │ - Slack discussion: https://nedap.slack.com/archives/C01/p1234
    │
  2 │ ## Description
    │ Tooltips stopped appearing after the portal refactor, and drags
    │ landed on the wrong menu entry when the list was scrolled.
    │
  3 │ ## Type of change
    │ - [ ] bugfix
    │ - [x] internal change
    │
  4 │ ## Checklist
    │ - [x] Tests added or updated
    │ - [ ] Changelog entry added
    │ - [x] Version label set
    │ - [ ] Migrations reviewed

METADATA
  5 │ REVIEWERS   none requested
  6 │ ASSIGNEE    @joris
  7 │ LABELS      internal change, version/minor
  8 │ BOARD       Moves Q3 (#7)
  9 │ DRAFT       yes
 10 │ BASE        master

── body matches template · 0 placeholders · nothing pushed or created ──
```

Show the whole thing. If it's long enough to scroll, that's fine — it's shorter than the PR it describes. Never truncate a checklist or elide a section with `…` to keep the card compact; a preview the user has to scroll is strictly better than one they have to trust.

Then the command line:

```
edit N · regen N · skip N · unskip N · title <text>
+@user  -@user  ·  reviewers none        assign @user · assignee none
label +name / -name · labels none        board <name|none>
base <branch> · draft on|off             issues #12 #34 · slack <url>
diff · why N · cancel                    create  ← the only thing that touches GitHub
```

`reviewers none`, `labels none`, and `assignee none` clear a field outright. Users ask for this constantly — "request no-one for now" is a normal state for a draft — and making them subtract handles one at a time is friction with no purpose.

`why N` reports provenance for that field: which commit, issue, Slack thread, CODEOWNERS entry, or history statistic produced it. Answer honestly, including "I inferred this from the diff" and "this is a placeholder because I have no evidence."

#### Flags, and what doesn't belong in them

After the card, you may add flags — but only for things that change a field in this PR and that you cannot resolve yourself. A required label with no clear value, a template section whose answer isn't in the diff, an ambiguous base branch.

Not flags: unused dependencies, code smells, missing tests, branch-behind-master, refactors you'd suggest. That's a code review, and the user asked for a PR description. Volunteering it buries the two decisions they actually need to make. If something genuinely alarming turns up, one sentence after the card, no options attached.

At most **one** question. Two stacked questions — each with a "would you rather A or B" — turns a confirmation step into an interview and is the fastest way to make this skill annoying enough to stop using. If two things are ambiguous, pick a defensible default for the smaller one, apply it, and show it in the card where they can override it with a command.

### 6. Loop

Apply commands and re-render only what changed plus the footer, so the terminal doesn't fill with repeated full previews.

Two exceptions where the full card comes back:

- **Any edit that touches the body.** Rewrite `body.md`, re-run `check_body.py`, then render the full body again. Reporting the write as a diff of the file is not a preview — the user cannot approve a PR from a list of removed lines.
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
