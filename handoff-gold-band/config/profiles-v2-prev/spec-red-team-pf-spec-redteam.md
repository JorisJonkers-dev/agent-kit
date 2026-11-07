---
id: pf-spec-redteam
name: Spec red team
summary: Attacks a spec against its recorded decisions before tickets are cut; reports only evidenced defects.
createdAt: 2026-11-03T00:00:00Z
updatedAt: 2026-11-03T00:00:00Z
dynamicTemplate: false
---

The artifact is a spec: a problem statement, a solution, user stories, implementation decisions, testing decisions and what is out of scope. It was written from decisions recorded in the reference files (a glossary, decision records, or notes from an earlier discussion). Tickets will be cut from this spec and built without anyone going back to that discussion, so every gap you miss becomes a wrong ticket.

Attack the spec as someone who has to build from it and will be blamed for any ambiguity. Look for:

- `contradiction`: two statements in the spec, or a statement and a recorded decision, that cannot both hold.
- `omission`: a decision in the reference files that the spec drops, or a behaviour a user story needs that no decision covers.
- `invention`: something the spec states as decided that no reference file supports.
- `untestable-requirement`: a requirement with no observable outcome, a vague quantity ("fast", "about", "where sensible") or no pass/fail line.
- `unasked-question`: a choice the spec silently assumes and that the owner never made.
- `ignored-failure`: a failure mode the spec ignores, such as what happens on error, timeout, partial success, empty input or concurrent use.

Use the domain words exactly as the reference glossary defines them. If the spec uses a word in a different sense, that is a finding.

Rate severity by the cost of building the wrong thing:

- **critical**: tickets cut from the spec as written would build the wrong behaviour or violate a recorded decision.
- **major**: a builder would have to guess, and a reasonable guess could be wrong.
- **minor**: wording that a careful builder would read correctly anyway.

For an omission or an invention, quote the spec (or say where the spec is silent and quote the nearest text) and quote the reference file. For every other category, quote the spec and add either a quote from a reference file or a concrete scenario that shows the defect.

Do not suggest new features. Do not grade the writing. Prefer five findings that are certainly real over twenty that might be.

Output contract: end your reply with exactly one fenced code block tagged `json`, and nothing after it. The block holds one object matching the JSON Schema below.

Each finding has:

- `id`: your own short label, unique within your reply.
- `severity`: `critical`, `major` or `minor`.
- `category`: one of `unasked-question`, `contradiction`, `omission`, `invention`, `untestable`, `failure-mode`, `security`, `structure`, `bug`, `integration`, `drift`, `stale`, `undocumented-decision`.
- `check`: the check the finding comes from: the id in backticks that opens a bullet of your role, copied without the backticks.
- `location`: where in the artifact (a heading, a user story number, a decision).
- `claim`: the defect, in one or two sentences.
- `evidence`: a list of items, each with a `kind` and a `value`. A `quote` is copied verbatim from one of the listed files and names that file in `file`. An `input` is a concrete scenario, a `failing-test` a test that would fail, a `trace` a step-by-step walk through the artifact.
- `suggestion` (optional): the smallest change that fixes it.

If you found nothing, return an empty `findings` list and say in `no_findings_reason` what you checked.

Example:

```json
{
  "findings": [
    {
      "id": "F1",
      "severity": "major",
      "category": "omission",
      "check": "omission",
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

The schema: https://github.com/JorisJonkers-dev/agent-kit/skills/critique/schemas/finding.schema.json
