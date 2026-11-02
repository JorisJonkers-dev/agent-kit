# Critique pipeline

Critics from other model families review each artifact of the spec-to-ticket
flow read-only, each from a frozen **packet**, blind to each other and to your
conversation. The agent you work in (the **orchestrator**) checks each
finding's evidence and rules on it. Bold words are defined in
[CONTEXT.md](../../CONTEXT.md); the detail behind this page is in
[integration.md](../../docs/critique-pipeline/integration.md).

`/flow` says where the current feature stands and its next step. It runs a
critique stage when you say yes, and names the Pocock command when only you
can type it.

Nothing is sent until the repository has a **family allowlist** entry
(per machine, `~/.config/agent-kit/critique-allowlist.json`, keyed by the
remote). A repository with no remote or a `nedap` remote never sends. An
entry covers documents only until its code permission is on, which the code
stages need, and leaves out commit messages and issue text until its history
permission is on. Both are off by default. The first stage that needs one
offers it.

## Stages

| Stage | Skill | After / before | Base critics (role) | Deep adds | You get |
|---|---|---|---|---|---|
| `decisions` | `/critique-decisions` | `/grill-with-docs` or `/grill-me` / `/to-spec` | Mistral Large, Gemini Flash (`gap-critic`) | GLM | Up to seven questions about decision branches nobody opened |
| `spec` | `/critique-spec` | `/to-spec` / `/to-tickets` | Mistral Large, Gemini Flash (`spec-redteam`) | GLM, DeepSeek Flash | Spec fixes; a question for any fix that changes a decision; a second round after more than three accepted critical findings |
| `tickets` | `/check-tickets` | `/to-tickets` / first `/implement` | DeepSeek Flash (`ticket-linter`) | Mistral Large | A script's check for cycles, dangling blockers and uncited stories, and structural fixes; scope changes become questions |
| `ticket-review` | `/review-ticket` | `/code-review` / the commit | Gemini Flash (`diff-reviewer`) | Mistral Large | For a risky staged change: each bug proven by a failing test, fixed, and kept as a regression test |
| `feature-review` | `/flow finish` | every ticket landed / the PR | Mistral Large, Gemini Flash (`feature-reviewer`) | GLM, DeepSeek Flash | A follow-up ticket per accepted finding, and the gate line |

Planning is the first three stages; implementation is one `/implement` per
ticket, each in a fresh session; finishing is the feature review.
`/flow no-critique` skips every stage on purpose, for trivial work.

A ticket review starts with **triage**: the change is risky when a path
matches routing's `triage.risk_paths` (auth, crypto, migrations,
concurrency, API surface, infra), more than 300 lines changed, or the ticket
is labelled `risk:high`; otherwise Mistral Small answers yes or no. A
low-risk change skips the review, and the skip is logged.

**Evidence decides.** A quote found in no packet file rejects its finding as
fabricated. A finding with no checkable evidence is a hint, reported and
never acted on. At the code stages a finding is accepted only on a failing
test the orchestrator ran or a trace it walked.

## Lenses

The angle a critic reviews from, one or two per critic in routing; full text
in `lenses/`.

- `fidelity`: every decision carried out, nothing undecided added.
- `testability`: every requirement has a test that can pass or fail.
- `failure-modes`: what happens when a dependency fails, hangs or returns
  nothing.
- `security`: what data leaves the machine or repository, and to whom.
- `integration`: whether assumptions about existing interfaces, layout,
  configuration and versions hold.
- `operability`: how the owner learns it failed, and what state it leaves.
- `simplicity`: mechanisms no user story needs, or two for one problem.
- `unasked-questions`: choices made without saying who decided.

## Depth

Base runs a stage's base critics; deep adds its deep critics. The runner
chooses from signals in the artifact, each firing when its value is above
the threshold:

| Stage | Deep when |
|---|---|
| `decisions` | more than 40 decisions or 4000 words, or a risk term |
| `spec` | more than 30 user stories or 5000 words, a risk term, or `risk:high` |
| `tickets` | more than 10 tickets, a risk term, or `risk:high` on the spec |
| `ticket-review` | more than 600 changed lines or 3 risk-path hits, or `risk:high` |
| `feature-review` | `risk:high` on the spec |

Risk terms are auth, migration, payment, crypto, encryption, concurrency and
`public API`, with common variants, as whole words. Pass `base` or `deep` to a
stage skill to choose yourself; the summary's depth line names the signals
that fired either way.

## Budget and quota

- **Per critic**: a price cap and a time limit. Vibe checks its cap between
  turns, so a critic can overshoot by one turn; where Vibe's figure can't be
  trusted, the runner charges the cap.
- **Per flow**: Vibe spend capped at $5 (`budget.vibe_flow_cap_usd`),
  triage included. Each flow key is capped on its own: the decisions stage
  charges the draft's, every later stage the spec's. Past the cap, Vibe
  critics don't launch and show as `budget-skip`.
- **Antigravity per day**: 20 invocations a UTC day
  (`antigravity_quota.daily_cap`). After 10 (`reserve_threshold`), the rest
  are kept for the spec and feature-review stages; other stages drop Gemini
  with the reason `quota` and use its fallback. Antigravity reports no cost.
- `/flow budget` shows the flow's Vibe spend, today's Antigravity count
  against the cap and reserve, and which stages would fall back now. The
  fallback it names is routing's alone; a real run also weighs the
  allowlist, your family and readiness, so it may pick a later one or leave
  the slot empty.

## Gates and the kill switch

- **Commit gate**: a risky staged change needs a passing `/review-ticket` run
  for that exact staged change, matched by a hash taken at record time, fixes
  and kept tests included, so a change restaged or amended after its review
  needs another. A review fails ("do not commit") while an accepted critical
  finding is unfixed; a degraded review passes, so an outage blocks nothing.
- **PR gate**: the feature review opens with `GATE: PASS` or
  `GATE: FAIL (<n> critical)`. It fails while an accepted critical finding,
  from this or an earlier feature review, has an open follow-up ticket. A
  gate holds for the head it reviewed: close each follow-up once its fix is
  on the feature branch, then `/flow finish` again.

[ADR-0002](../../docs/adr/0002-exactly-three-critique-hooks.md) allows three
hooks, for Claude Code and Codex only: `critique-commit-gate` (before
`git commit`), `critique-pr-gate` (before `gh pr create`, or a push outside
the feature branch patterns) and `critique-session-start` (one line
suggesting `/flow setup` when this machine's setup is missing or failing).
They do nothing in a repository that isn't opted in, call no model or
network, let the command through on their own errors, and block only on an
explicit FAIL, with one line naming the skill to run. Setup installs them
into Claude Code's `settings.json` and Codex's `hooks.json`, merged by name
beside your own hooks; Codex runs them once you trust them in `/hooks`, and
`/flow setup`'s "hooks installed" item checks that. The runner's `gate`
command decides both gates.

**Kill switch**: `CRITIQUE_OFF=1` in the environment, or
`{"disabled": true}` in `~/.config/agent-kit/critique.json` (the per-machine
critique configuration), turns every hook into a no-op; `/flow setup`'s
kill-switch item writes or clears that flag. A stage you start yourself
still runs.

## Routing

`routing.json` beside this file holds the critic clients (CLI, model,
family), each stage's base and deep rosters (client, role, lenses, price cap,
time limit), the `fallbacks`, the budget, the Antigravity quota, triage, the feature
branch patterns a push may update without a feature review (`gate`) and the
stats thresholds. Model names live only there and in the
`critic-<model>` Vibe agent files.

No critic shares the orchestrator's family, and a stage runs one critic per
family. A critic that can't run (family not allowed, CLI missing or signed
out, quota) gives its slot to the first fallback that can: Mistral Large
falls back to GLM, then Gemini Flash; Gemini Flash to GLM. With none left the
stage runs with fewer critics, and with no critics at all it is reported
degraded and the flow goes on.

**Retune one machine** in `~/.config/agent-kit/critique-routing.json`, a JSON
merge patch over `routing.json`: name only what changes, `null` removes a
key, and a list is replaced whole. A bad merge is `invalid-routing`, naming
both files.

```json
{ "budget": { "vibe_flow_cap_usd": 8 }, "antigravity_quota": { "daily_cap": 50 } }
```

A default for every machine is a change to agent-kit's
`skills/critique/routing.json`.

**Decide what to change** with `/critique-stats`: from the ledger
(`~/.local/share/agent-kit/critique/ledger.jsonl`, metadata only) it reports
each critic client's precision, unique catches, fabrication, cost and
failures per stage. It suggests a swap only on 20 or more adjudicated
findings, flags a stage with no unique catch across its last three features,
and never edits routing.
