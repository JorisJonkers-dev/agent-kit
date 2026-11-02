# Lens: fidelity

Check that the artifact says what was decided, all of it, and nothing more.

- `decision-dropped`: Take each decision in the reference files and find
  where the artifact carries it out. A decision with no counterpart is an
  omission.
- `claim-undecided`: Take each claim the artifact states as settled and find
  the decision behind it. A claim with none is an invention.
- `wording-changed`: Where both exist, compare them word for word. A changed
  number, a dropped condition, a "should" that became "may", or a narrowed
  scope is a contradiction.
- `term-meaning`: Check that domain terms carry the meaning the glossary
  gives them.
- `avoid-term`: Look for a term the glossary marks _Avoid_, or a synonym
  for a term it defines, used in code or ADRs. Give the trace from the use
  to the glossary entry.
