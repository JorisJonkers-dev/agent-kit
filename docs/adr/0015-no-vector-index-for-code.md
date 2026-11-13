# No vector index for code

Agents in this estate are given the code-nav server (`search`, `outline`,
`expand`, `map`, `symbols`, `related_repos`) over the live working tree, with
each repository's `AGENTS.md` for orientation. We do not build a vector
(embedding) index of code, and we do not add any index before a measurement
says it is needed. Spec #392 story 160 asks for this to be recorded with the
conditions for revisiting it.

Why not an index of vectors:

- **Stale answers.** The code-nav server reads the working tree on each call and
  keeps nothing, so a result is never older than the file. A vector index is a
  copy that goes stale on every commit, branch switch and uncommitted edit.
  Keeping it fresh needs a watcher or a build step per repository, on two
  operating systems, in every instance the roots allow.
- **A model and a place for it to run.** Embeddings need an embedding model:
  one from a provider (a new spend line, and the code leaves the machine, which
  the repository rule for the work instance forbids) or one run locally on the
  workstation GPU, which the Mac and the runner image do not have. ADR-0006
  rejected an embedding index for critics on the provider or local model and
  reproducibility grounds.
- **Results that cannot be replayed.** A symbol, a path and a reference are
  exact. Nearest-neighbour results depend on the model version and the chunking,
  so a question set run twice does not give the same files, and the frozen
  set (B7-02) is how every retrieval change is measured.
- **The measurement does not show a need.** The first record
  (docs/code-nav-measurements.md) is a partial run: it covered one question, FR-02, on two
  cheap models. It shows no layer helping or hurting, and it has no concept
  question in it, so it cannot support an index. An index is paid for in
  upkeep and is added only when a number says so.

If a gap is found, the order is fixed by spec #392 (story 159): first BM25 over
identifiers, doc comments and paths, keyed to git for freshness (ticket B7-13,
#528); dense vectors only after that, on the workstation GPU.

## Conditions for revisiting

This decision is reopened only by a new measurement, never by preference. A
measurement is a run of the whole frozen question set through the runner, at
least three runs a question, on at least two model families, at every layer of
docs/code-nav-measurements.md, judged at the best one. Reopen it when any of these
holds:

1. **Concept questions miss more than 15 percent.** The concept family (questions
   asked as ideas, not names) is answered wrong, or without its gold files, in
   more than 15 percent of runs. This does not reopen the decision by itself: it
   starts B7-13 (#528), the identifier index, first.
2. **The identifier index was built and did not help.** The concept miss rate
   is still above 15 percent after B7-13, so the gap is vocabulary and not
   lookup. That is the evidence a vector index needs, and it reopens this
   decision.
3. **Reference misses do not point at an index.** Misses in the refs family
   (who calls, who uses; the owner sets the threshold when the full set is run)
   trigger B7-12 (#527), the language-server bridge. They
   are not a reason for a vector index: references are exact, and an
   embedding finds them worse than a language server does.
4. **The cost of looking dominates.** Tokens spent exploring (the transcript
   analyser, B7-01) stay above the budget the owner sets even at the best layer by that measurement
   with the concept miss rate under 15 percent. That reopens this decision only after
   the map and outline have been tried, since those are the cheaper fix.
5. **The constraints move.** A local embedding model runs on every host the
   server is installed on, or the repository rule lets code reach a provider's
   embedding endpoint, and a held-out question set shows the index winning on
   tokens and accuracy together.

A run of fewer than the full set, fewer than three runs, or one model family
does not meet the bar to reopen, or to close, the decision. The first record is
such a run.

## Consequences

- code-nav stays read-only, index-free and without a daemon or a cache to
  invalidate.
- The two conditional tickets, B7-12 (#527) and B7-13 (#528), start only from a
  full-set measurement. The first record does not start either.
- A measurement run is appended to docs/code-nav-measurements.md with its raw
  rows under `evals/code-nav/results/`, so the evidence for reopening is in the
  repository.
