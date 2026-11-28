### 4. Draft

Fill the template. Where a fact is not in evidence — test results you haven't seen, a benchmark number, a ticket ID, a screenshot, a migration rollback plan — insert a placeholder rather than a plausible sentence:

```
«benchmark: p95 before → after»
```

Placeholders are a feature. Count them and surface the count in the preview. Invented specifics are the one failure mode that makes this skill worse than writing the PR by hand, because they read as verified.

**Fill the template; don't improve it.** Four rules, all of them checked mechanically by `pr-composer check-body` in the next step:

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
pr-composer check-body \
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

