# Output contract

Write whatever reasoning you need first. Then end your reply with exactly one
fenced code block tagged `json`, and nothing after it. The block holds one
object that matches the JSON Schema below.

Each finding has:

- `id`: your own short label, unique within your reply (`F1`, `F2`, ...).
- `severity`: `critical`, `major` or `minor`.
- `category`: one of `unasked-question`, `contradiction`, `omission`,
  `invention`, `untestable`, `failure-mode`, `security`, `structure`, `bug`,
  `integration`.
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

The schema:
