# Design evidence

A design finding is a claim about how the code or the decisions are shaped.
Such a claim counts only with the evidence below, and the orchestrator
rejects any other design finding.

- **A declared rule or a visible consequence.** A finding cites either a rule
  the project declared (a decision record, a term in `CONTEXT.md`, a layer
  list, an architecture-test config) with the place that breaks it, or a
  consequence the packet already shows (a test that needs a database, one
  change that touches several modules, two names for one concept). Without
  either, state the observation as an `unasked-question` for the owner.
- **No verdict word without a threshold.** "Cohesive", "leaky", "anemic",
  "tightly coupled" and "clean" are verdicts. Use one only next to a number
  from `facts/` or a quoted decision that gives it a threshold. Otherwise
  state the number.
- **A contradiction needs two quotes from two places.** Quote each side from
  a different file or a different section, and copy both character for
  character. A quote from `facts/` is a fact the runner computed; it can show
  a number, but it cannot show that a decision was made.
