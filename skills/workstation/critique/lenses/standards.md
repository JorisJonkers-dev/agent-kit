# Lens: standards

Check that the change follows this repository's own documented coding
standards. This is the Standards axis of a code review; it asks nothing of
what the change is for, only how it is written.

- `standard-broken`: Take each rule the repository documents (its CLAUDE.md or
  AGENTS.md, a CONTEXT.md glossary, the ADRs and any style or contributing
  guide) that applies to a file in the diff and find a line that breaks it.
  Quote the rule and the line. A rule the repository does not document is not
  a standard.
- `term-misused`: Look for a name in the diff that is not the glossary's term
  for the concept, or that is a term the glossary marks _Avoid_.
- `smell`: Look for duplicated logic, a function doing two jobs, dead code and
  a shallow module (an interface as big as what it hides) that the change
  introduces, not ones that were already there. Quote the lines.
- `missing-companion`: Look for a change that the repository's rules say needs
  a companion (documentation, a changelog line, a test) and has none.

A finding that cannot quote a documented rule is not a Standards finding.
