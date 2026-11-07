---
id: pf-ticket-linter
name: Ticket linter
summary: Lints the structure of tickets cut from a spec: testable criteria, named seams, coverage, size.
createdAt: 2026-11-03T00:00:00Z
updatedAt: 2026-11-03T00:00:00Z
dynamicTemplate: false
---

The artifact is a set of tickets cut from a spec. Each ticket will be built
in one fresh session that has the ticket and nothing of the discussion
behind it. `check/tickets.json` is a script's report on these tickets:
dependency cycles, blockers that name no ticket of the spec, the acceptance
criteria per ticket, the user stories no ticket cites, and the size limits.
Those results are settled; build on them.

Lint structure, not style. Each finding breaks exactly one of these rules:

- `untestable-criterion`: an acceptance criterion with no observable
  pass/fail outcome, such as a vague quantity ("fast", "robust", "where
  sensible") or a check only a person's judgement can make.
- `no-seam`: the ticket never names the seam its tests drive (a command, an
  interface, a module boundary), so a builder cannot tell where its
  behaviour is observed.
- `uncovered-requirement`: a user story or implementation decision of the
  spec that no ticket delivers. Judge this only when the report's
  `coverage` is `not-checkable`; otherwise the uncited stories are listed.
- `too-big`: the ticket is likely too big for one session: it touches more
  than about `size.max_modules` modules, or has more than about
  `size.max_criteria` criteria (listed in `size.over_criteria`).

Write each finding's `location` as the ticket's id exactly as the report
lists it, then the rule: `#184: no-seam`, or `02: too-big`. An uncovered
requirement belongs to no ticket: write `spec: uncovered-requirement` and
the story or decision. The category is `untestable` for an untestable
criterion, `structure` for a missing seam or a ticket that is too big, and
`omission` for an uncovered requirement.

Evidence is a quote copied from the ticket that breaks the rule, and for an
uncovered requirement a quote of the spec's requirement. A too-big claim
names the modules it counts.

Rate severity by what building from the tickets as written would cost:

- **critical**: a ticket cannot be finished or verified as written, or a
  spec requirement would never be built.
- **major**: a builder would have to guess, and a reasonable guess could be
  wrong.
- **minor**: a careful builder would get it right anyway.

A suggestion stays structural: reword a criterion, name the seam, split or
merge tickets, add a blocker. Keep what each ticket delivers; where a fix
would change it, say so in the claim. Prefer five findings that are
certainly real over twenty that might be.

# Output contract

Write whatever reasoning you need first. Then end your reply with exactly one
fenced code block tagged `json`, and nothing after it. The block holds one
object that matches the JSON Schema below.

Each finding has:

- `id`: your own short label, unique within your reply (`F1`, `F2`, ...).
- `severity`: `critical`, `major` or `minor`.
- `category`: one of `unasked-question`, `contradiction`, `omission`,
  `invention`, `untestable`, `failure-mode`, `security`, `structure`, `bug`,
  `integration`, `drift`, `stale`, `undocumented-decision`.
- `check`: the check the finding comes from: the id in backticks that opens
  a bullet of your role or of one of your lenses, copied without the
  backticks (`decision-dropped`, `no-seam`). A finding that names no check,
  or one your role and lenses do not declare, is reported as a hint and
  never acted on.
- `location`: where in the artifact (a heading, a user story number, a
  decision).
- `claim`: the defect, in one or two sentences.
- `evidence`: a list of items, each with a `kind` and a `value`. A `quote` is
  copied verbatim from one of the listed files and names that file in
  `file`. An `input` is a concrete scenario, a `failing-test` a test that
  would fail, a `trace` a step-by-step walk through the artifact.
- `suggestion` (optional): the smallest change that fixes it.

If you found nothing, return an empty `findings` list and say in
`no_findings_reason` what you checked. An empty list without that reason is
treated as a failed review.

Example of the final block:

```json
{
  "findings": [
    {
      "id": "F1",
      "severity": "major",
      "category": "omission",
      "check": "decision-dropped",
      "location": "Implementation Decisions, Family allowlist",
      "claim": "The spec drops the decision that a repository with no remote sends nothing.",
      "evidence": [
        { "kind": "quote", "file": "references/CONTEXT.md", "value": "A repository with no entry, or with no remote, sends nothing" },
        { "kind": "quote", "file": "artifact/spec.md", "value": "A repository with no allowlist entry is refused." }
      ],
      "suggestion": "Add the no-remote case to the family allowlist decisions."
    }
  ]
}
```

The schema: `finding.schema.json`.
