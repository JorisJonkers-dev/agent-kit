# House style profile (12 merged PRs)

## Title

- conventional-commit prefix: **91.7%**
- ticket key prefix: **25.0%** (keys: ABC, TICKET)
- both together: 25.0%
- length: median 17, p90 24 chars
- lowercase after prefix: 75.0% · trailing period: 16.7%
- scopes: api (2), db (1), core (1), github (1)

Recent titles:
  - feat(api): add endpoint.
  - [ABC-12] fix(db)!: Repair index
  - docs: update readme
  - Update things
  - ABC-99 chore: bump
  - feat: lowercase title.

## Body

- empty bodies: **8.3%**
- length: p10 6 / median **34** / p90 34 words
- template HTML comments left in: 50.0%
- checklists: 58.3% (boxes actually ticked: 58.3%)
- images/screenshots: 16.7% · Slack links: 8.3%
- issue references: 66.7% · preferred keyword: **closes**

Sections by frequency:
  - summary — 83.3%
  - test plan — 66.7%
  - notes — 50.0%
  - risks — 8.3%

Typical order: summary → test plan → notes

Sections usually left hollow (fill these only if you have real content):
  - notes — 100.0% hollow of 6 seen
  - summary — 40.0% hollow of 10 seen

## People

- reviewers per PR: median **1** (mean 1.25)
- frequent reviewers: @alice (6), @bob (4), @carol (2), @dave (1), @team-x (1), @s1 (1)
- frequent approvers: @alice (5), @bob (2), @carol (1)

Reviewers by area:
  - `api/x.rs` → @alice (2), @bob (1), @carol (1), @dave (1)
  - `api/y.rs` → @alice (1), @bob (1), @carol (1), @dave (1)
  - `README.md` → @alice (1), @bob (1), @carol (1), @dave (1)
  - `api/z.rs` → @alice (2), @carol (1)
  - `db/a.sql` → @alice (1), @bob (1)
  - `ci/x.yml` → @bob (1), @alice (1)
  - `web/index.html` → @carol (1), @alice (1)
  - `ci/y.yml` → @bob (1)
  - `Cargo.toml` → @bob (1)

## Meta

- **`type` label appears on 91.7% of merged PRs — treat as mandatory.** Values: `type: feat` (2), `type: fix` (2), `type: docs` (1), `type: chore` (1), `type: refactor` (1), `type: test` (1)
- labelled PRs: 91.7% · type: feat (2), type: fix (2), area: api (1), type: docs (1), area: docs (1), type: chore (1), version/minor (1), type: refactor (1)
- median size: 12 lines across 2 files
- base branches: main (11), develop (1)
