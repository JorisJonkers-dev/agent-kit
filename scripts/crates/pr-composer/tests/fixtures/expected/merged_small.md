# House style profile (3 merged PRs)

> **only 3 merged PRs — treat every rate here as anecdote, lean on the template and conventional defaults instead**

## Title

- conventional-commit prefix: **100.0%**
- ticket key prefix: **33.3%** (keys: ABC)
- both together: 33.3%
- length: median 24, p90 24 chars
- lowercase after prefix: 100.0% · trailing period: 33.3%
- scopes: api (1), db (1)

Recent titles:
  - feat(api): add endpoint.
  - [ABC-12] fix(db)!: Repair index
  - docs: update readme

## Body

- empty bodies: **33.3%**
- length: p10 19 / median **26** / p90 19 words
- template HTML comments left in: 33.3%
- checklists: 33.3% (boxes actually ticked: 33.3%)
- images/screenshots: 33.3% · Slack links: 33.3%
- issue references: 66.7% · preferred keyword: **closes**

Sections by frequency:
  - summary — 66.7%
  - test plan — 66.7%
  - notes — 33.3%
  - risks — 33.3%

Typical order: summary → test plan → notes

## People

- reviewers per PR: median **2** (mean 2.67)
- frequent reviewers: @alice (2), @bob (2), @carol (1), @dave (1), @team-x (1), @s1 (1)
- frequent approvers: @alice (2), @bob (1)

Reviewers by area:
  - `api/x.rs` → @alice (1), @bob (1), @carol (1), @dave (1)
  - `api/y.rs` → @alice (1), @bob (1), @carol (1), @dave (1)
  - `README.md` → @alice (1), @bob (1), @carol (1), @dave (1)
  - `db/a.sql` → @alice (1), @bob (1)

## Meta

- **`type` label appears on 100.0% of merged PRs — treat as mandatory.** Values: `type: feat` (1), `type: fix` (1), `type: docs` (1)
- labelled PRs: 100.0% · type: feat (1), area: api (1), type: fix (1), type: docs (1), area: docs (1)
- median size: 4 lines across 1 files
- base branches: main (3)
