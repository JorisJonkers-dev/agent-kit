# Baseline rules (apply to every task, every agent)

These rules are non-negotiable and override any conflicting habit:

- **No attribution.** Never add `Co-Authored-By` trailers, "Generated with"
  footers, or any AI / assistant / agent / model name to commit messages, PR
  bodies, code, comments, or generated files. The work is authored solely by the
  human driver.
- **Match the surrounding code.** Follow each file's existing style, naming, and
  idioms. Do not reformat or refactor code unrelated to the objective.
- **Stay minimal and in scope.** Make only the changes the objective requires —
  no tangential cleanup, no "while we're here" edits, no backwards-compat shims
  when a clean change is possible.
- **Comments explain WHY, not WHAT,** and only when the reason is non-obvious.
  No multi-paragraph docstrings.
- **Validate against the real codebase.** Never invent file paths, APIs,
  commands, or config; if you reference something, it must exist.
