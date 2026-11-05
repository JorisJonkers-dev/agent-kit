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
remote). A repository with no remote never sends. A `nedap` remote needs no entry and
has a work-only roster instead: Codex on the nedap account, only under a live
model-gateway grant for the `workflow` the call names, and the work Claude
agent, each never in the orchestrator's own family, and no other family. Any
other repository with no entry reviews under the personal default — every
family a critic can come from, code but no history — and a recorded entry
always wins over the default. An entry covers documents only until its code
permission is on, which the code stages need, and leaves out commit messages
and issue text until its history permission is on. Both are off by default.
The first stage that needs one offers it.

## Stages

| Stage | Skill | After / before | Base critics (role) | Deep adds | You get |
|---|---|---|---|---|---|
| `decisions` | `/review-grill` | `/grill-with-docs` or `/grill-me` / `/to-spec` | Mistral Large, Gemini Flash (`gap-critic`); with a decision record among the decisions, DeepSeek Flash (`adr-reviewer`, `adr-record`) too | GLM | Up to seven questions about decision branches nobody opened |
| `spec` | `/review-spec` | `/to-spec` / `/to-tickets` | Mistral Large, Gemini Flash (`spec-redteam`) | GLM, DeepSeek Flash | Spec fixes; a question for any fix that changes a decision; a second round after more than three accepted critical findings |
| `tickets` | `/review-tickets` | `/to-tickets` / first `/implement` | DeepSeek Flash (`ticket-linter`) | Mistral Large | A script's check for cycles, dangling blockers and uncited stories, and structural fixes; scope changes become questions |
| `ticket-review` | `/review-implementation` | `/code-review` / the commit | Gemini Flash (`diff-reviewer`) | Mistral Large | For a risky staged change: each bug proven by a failing test, fixed, and kept as a regression test |
| `feature-review` | `/flow finish` | every ticket landed / the PR | Mistral Large, Gemini Flash (`feature-reviewer`) | GLM, DeepSeek Flash | A follow-up ticket per accepted finding, and the gate line |
| `adr-review` | `/critique-adrs` | on demand, outside any feature | Mistral Large with `adr-reviewer` and `adr-record`, Gemini Flash with `adr-reviewer` and `decision-log` | GLM with `adr-reviewer`, both lenses | Editorial fixes to the records; a question for any fix that would change a decision, and a superseding record drafted from your answer; a second round after more than three accepted critical findings |

Planning is the first three stages; implementation is one `/implement` per
ticket, each in a fresh session; finishing is the feature review.
`/flow no-critique` skips every stage on purpose, for trivial work.

`adr-review` is a **review stage**: it runs over a repository's decision
records when you ask for one, never as a step of a feature's flow. Each run
is a flow of its own, keyed by its run id, with its own $5 Vibe cap, so a
review never draws on a feature's budget. Its packet holds the records under
review in `artifact/`, and the glossary and the other records in
`references/`. With the code permission off it sends those documents only
and skips the checks that need the code: `drift`, `stale` (against the
code) and `adr-record`'s `no-confirmation`. A finding of one is a hint
(`needs-tree`), and the report and the summary name each skipped check.
The runner's own pre-check, `check/adrs.json`, covers the repository either
way: for each record in force it lists `drifted` (a path or symbol named in
backticks that the working tree no longer holds; a symbol needs an internal
capital, as `computeWindow`, or a `path#symbol` anchor) and `stale` (the record's
decision date, the first `YYYY-MM-DD` on a `Date:` line, a bold `**Date**:`
or list-item date line, or a frontmatter `date:` key, precedes the last commit
that changed a path it names). Both are
computed facts for the owner, not errors: they never make `check-adrs`
fail.
`critique check-plan --json '{"plan": "refactoring-plan.md"}'` (or with `"repo"`,
and `plan` relative to it) checks a refactoring plan and exits 3 while it breaks
a rule. The plan holds the sections `## Goal`, `## Target`, `## Safety net`,
`## Steps`, `## Not doing` and `## Open decisions`. Each step is a
`### S01: <title>` heading followed by `- **Field**: value` lines: Kind,
Technique, Anchor, Touches, Safety net, Main stays green because, Rollback,
Transitional element, Blocked by, Done when. Kind is exactly one of
characterize, tidy, structure, behaviour, expand, migrate, contract,
remove-transitional. An anchor is a backticked path (a fact row, with an
optional `#symbol`), an `ADR-0005` id, or `prerequisite-of:S02`. File paths
appear only in anchors, in Touches and in the Goal's evidence. The report is
`{ status, errors: [{ step?, rule, message }], stored_data_steps, steps, notes }`;
`stored_data_steps` lists the migrate and contract steps that touch a table,
column, migration, schema, database or stored data. Step size limits come from
`ticket_size` in routing.
`critique facts --json '{"repo": ..., "files": [...], "modules": [...]}'`
prints the runner's deterministic facts at HEAD's commit (ADR-0004): for each
named file its last commit, that commit's own committer date and its churn
(how many commits changed it), and for each named module (a file path) its
imports, fan-in, fan-out and instability over the named set, and the import
cycles among them. It holds no author and no commit message, gives the same
bytes for the same commit, sends nothing and writes nothing. A module in a
language the scan does not know (TypeScript/JavaScript, Python, Kotlin, Go and
Rust are) gets `language: null` and a note in `notes`.
With it on, the packet is a **repository packet**: it adds `tree/`, a sample
of the code chosen by rule (the files the records name, then the top
hotspots, each module's entry file, then the files on each import cycle),
`tree/MANIFEST.tsv` listing every tracked path with its line count, and
`facts/`, whose `INDEX.md` records every file the cap dropped. It names a
dropped file only when the records name it or it is a top hotspot; it counts
the other drops per directory and reason, with their bytes, so the index
stays small however many files are left out. Past its own cap,
`tree/MANIFEST.tsv` counts its least-ranked paths per directory at its end
and says how many. Vendored,
generated and lock files are never sent. A long file is sent in windows from
its first line, so `tree/<path>:<line>` keeps its number, and a citation
outside what the packet holds is rejected. A quote from `facts/` is a fact,
never a recorded decision. `/flow reviews` lists recent review
runs with their verdicts and spend, and never calls a review due.

A ticket review and a feature review send a **code packet**, computed at the
reviewed commit by rule. Its diff leaves out the hunks of every vendored,
generated and lock file (the same `exclude` globs and "do not edit" marker),
keeping one line that names each and says why. `tree/REPO-MAP.md`, named
first in the prompt, lists every TypeScript, JavaScript and Python file with
its top-level symbols and their signatures, ranked by how many other files
import the file and name the symbol. The prompt then lists **context
slices** instead of whole files: each changed function or block, the
symbols it calls, and the functions that call it, found through the import
and reference scan. `tree/` is capped like a repository packet: the slices
go in first, then the changed files, their importers and the rest of the
tree, in windows that keep their line numbers. A file cut short holds only
those lines, a run it does not hold is blank but for a marker, and a
citation of a line outside them is rejected. `facts/INDEX.md` records every
slice and every file the cap dropped or cut short: by name for the changed
files, their importers, the files holding a slice and any file the diff, a
ticket or the spec names, and counted per directory and reason, with their
bytes, for the rest of the tree.

A diff too big to fit beside the tickets, the spec, the repo map and the
facts index is sent in windows by file instead of refused: the changed
source files, then the tests, then the docs, each group by changed lines,
most first. It takes at most half the room those documents leave, so the
tree keeps room for the slices. Each hunk keeps its `@@` line, so a line
it shows can be cited as `tree/<path>:<line>` even where the tree does not
hold it; `facts/INDEX.md` lists each hunk left out by file and line, and a
citation of a line only a left-out hunk shows is rejected unless the tree
holds it. A deleted or emptied file's hunks are cited at their lines at
the base. When listing every drop it names would not fit, the facts index
takes a quarter of the room and ends each list of names with a line
counting what it leaves unlisted; the per-directory counts stay. Prepare still refuses with `packet-over-cap` when the
tickets, the spec, the repo map and the facts index's own text pass the
cap.

A ticket review starts with **triage**: the change is risky when a path
matches routing's `triage.risk_paths` (auth, crypto, migrations,
concurrency, API surface, infra), more than 300 lines changed, or the ticket
is labelled `risk:high`; otherwise Mistral Small answers yes or no. Lines of
vendored, generated and lock files are not counted, and Mistral Small reads
the same filtered diff. A low-risk change skips the review, and the skip is
logged.

**Evidence decides.** A quote found in no packet file rejects its finding as
fabricated. A finding with no checkable evidence is a hint, reported and
never acted on. At `adr-review` every finding quotes a record under review,
a contradiction quotes two distinct files (records or references, never a
fact), and wording with two readings gives both as an input. At the code stages a finding is accepted only on a failing
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
- `dependency-direction`: dependencies point inward, with the import cited.
- `domain-language`: one word, one meaning, and a named relation between contexts.
- `aggregates`: each invariant held by its root, and one transaction per aggregate.
- `coupling`: modules that depend on another's internals, or share one decision.
- `test-seams`: behaviour testable at a seam with no real database, network or clock.
- `quality-scenarios`: quality claims with a stimulus and a number, and a check for each structural decision.
- `adr-record`: one decision record at a time, for its content and never
  its headings: a sentence that states the decision, a rejected alternative
  with its reason, a cost, a context that does not presuppose the choice, a
  "because" tied to a stated force, dated outside claims, a confirmation
  where one is possible, one choice per record, and wording with only one
  reading. Each check fixes its severity.
- `decision-log`: the records as one log: records in force that contradict,
  a replacement with no status or link, and references nothing resolves,
  beyond what `check/adrs.json` reports. Documents only for now.

### Check ids

Every lens bullet and every role rule opens with a **check id**: a
lower-case slug in backticks, then a colon, at the start of a top-level
bullet.

```markdown
- `dependency-fails`: For each external dependency (a process, a network
  call, a file, a service), ask what the artifact does when it fails, ...
```

The pattern is `^- \`([a-z0-9]+(?:-[a-z0-9]+)*)\`(?: \((critical|major|minor)\))?:`;
the bracketed severity is described below. In a lens every bullet
is a check, and a note that is not one goes in a paragraph after the list.
In a role the rules are the bullets of its first list; later lists, such as
evidence kinds and severity levels, carry no id. An id is unique within its
file and stays stable once shipped: renaming one starts its stats afresh.

Each finding names the check it comes from in its `check` field. Fan-out
reads the ids from the critic's own role and lenses; a finding that names no
check, or one its critic's prompt does not declare, becomes a hint
(`no-check` or `unknown-check`), unless the stage's evidence rule already
made it one. A merged finding keeps the check, and is judged against the
prompt, of the critic that raised it first. Each ledger finding line carries
`check`, or `null` when the finding named none or named something that is
not an id. `tests/test_critique_check_ids.py` fails on a check bullet without
an id, or an id used twice in one file.

A check may fix the severity of its findings, in brackets between the id
and the colon:

```markdown
- `no-rollback` (critical): A data step with no rollback step ...
```

Fan-out then sets the severity of a finding that names the check, whatever
the critic wrote, and keeps it as `declared_severity`. The orchestrator may
override it on its verdict with `severity` and `severity_reason`, a short
slug such as `copy-only-change`; record refuses an override without a
reason, one equal to the declared severity, and one on a finding whose check
fixes none. The overridden severity is the one that counts, for a second
round among others. Each ledger finding line carries `declared_severity`,
`severity_override` and `severity_reason`, or `null`. Only the files on the
validator's `DECLARES_SEVERITY` list may declare a severity; the list is
today the `adr-record` lens, and the `plan-critic` role once it ships.

The `adr-record` checks and their fixed severities:

- `no-decision-sentence` (critical): no sentence states what was decided.
- `no-rejected-alternative` (major): no alternative turned down, or none
  with the reason it lost.
- `no-cost` (major): no cost, risk or negative consequence.
- `solution-in-context` (minor): a context that already states or argues
  for the choice.
- `no-because` (major): a choice with no "because" tied to a stated force.
- `undated-claim` (minor): a claim about the outside world with no date or
  source.
- `no-confirmation` (minor): a decision a test, lint rule or script could
  check, with no such check named. Needs `tree/`: a documents-only run
  skips it, and the critic and the report name it as skipped.
- `two-choices` (major): two choices in one record that could be reversed
  separately.
- `two-readings` (major): wording two readers would build differently, with
  both readings given.

The `adr-reviewer` role never argues that a chosen option is wrong and never
copy-edits; a record that fails the significance gate (`not-significant`) is
at most minor.


## Refute round

After you rule on the findings and before any fix or question, the runner's
`refute` command sends each accepted critical finding to one more critic.
Critical is the severity that counts: your verdict's `severity` override
when it gives one, else the severity fan-out set.
It is the first routed client of the stage, base before deep (each slot's
client, then its fallbacks), whose family neither raised the finding nor is
yours, that the allowlist permits and that can run now. It runs with the
`refuter` role, no lens, on the run's own packet; the finding goes in its
prompt, never in the packet, so a quote of the finding is no evidence.

The refuter answers in the finding format:

- no objection: an empty `findings` list with `no_findings_reason`;
- `refute` or `downgrade`: exactly one finding whose `check` names that
  rule of the refuter role, with the original category and location; a
  downgrade gives `major` or `minor` in `severity`.

Its evidence is checked like any finding's, with the stage's evidence rule.
A refutation that holds rejects the finding (`refuted`) when record runs; a
downgrade that holds lowers its severity and keeps it accepted. Anything
else leaves your verdict standing (`stands`, with the reason): a quote the
packet does not hold, nothing the stage counts as evidence, an answer
outside the three, or a refuter that failed or wrote to its packet. With no
eligible client the finding is `skipped`, and the skip is noted. The second
round counts only the criticals still accepted after the round.

Results go to `refute.json` in the run directory, a `## Refute round`
section of the report, and one `refute` line each in the ledger (the
finding, its check, the outcome and answer, and the refuter's client, model
and cost). A refuter's launch is logged like any critic's, so its spend
counts against the flow's Vibe cap, and agy's against the day's quota. A run
is refuted once, and only before it is recorded.

The stage skills run refute right after their verdicts, before any fix,
follow-up or owner question. They record a refuted finding as `rejected`
with `reason_code: "refuted"` themselves, since an accepted verdict at the
code stages needs a `follow_up` or `regression_test` that a refuted finding
never gets, and they name each skipped refute above the summary.

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
| `adr-review` | more than 10 records under review, or a risk term |

Risk terms are auth, migration, payment, crypto, encryption, concurrency and
`public API`, with common variants, as whole words. Pass `base` or `deep` to a
stage skill to choose yourself; the summary's depth line names the signals
that fired either way.

## Budget and quota

- **Per critic**: a price cap and a time limit. Vibe checks its cap between
  turns, so a critic can overshoot by one turn; where Vibe's figure can't be
  trusted, the runner charges the cap.
- **What a Vibe critic costs**: the runner computes it from the export's
  tokens at the client's `prices` in routing: uncached input at the input
  price, cached input at the cached price (10% of the input price, as
  Mistral bills it), output at the output price. That figure is what the
  ledger, Vibe's plan window, `/flow budget` and the reports use. Vibe's own
  `cost_usd` is kept beside it as `vibe_cost_usd`. A client with no prices,
  or an export with no token counts, falls back to Vibe's figure.
- **Vibe's plan limit**: Vibe is a subsidised plan, so it is counted against
  the plan's own windows, not a dollar cap of the runner's. Before a stage
  plans a Vibe critic it asks the model gateway for Vibe's plan limit — the
  windows this machine's `clients.vibe.plan_limits` declares, with what the
  machine-wide ledger says is spent in each. At or above the gateway's
  warning ratio (80%) the stage's summary carries the gateway's warning and
  the stage runs. At 100% of any window Vibe counts as unable to start, so a
  Vibe critic is dropped with the reason `quota` and the stage runs with the
  critics left. A fallback retry is checked again.
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

- **Commit gate**: a risky staged change needs a passing `/review-implementation` run
  for that exact staged change, matched by a hash taken at record time, fixes
  and kept tests included, so a change restaged or amended after its review
  needs another. A review fails ("do not commit") while an accepted critical
  finding is unfixed. A degraded review, in which no critic reported, does
  not pass either: the commit stays blocked until a `/review-implementation` run in
  which a critic reported, or the kill switch.
- **Merge commits**: the commit gate also runs before a `git merge` or
  `git pull` that may record a merge commit, and before a `git commit` (or
  `git merge --continue`) while `MERGE_HEAD` is set. A merge goes through
  when every commit it brings in has a passing, non-degraded `/review-implementation`
  record (matched by the reviewed commit, or by the staged change it was made
  from) or was low-risk on its own, and the merge adds nothing beyond the
  clean auto-merge except manifest sha256 pins and rebuilt generated files
  (routing's `packet.exclude.generated` globs, or a "do not edit" marker).
  Otherwise its own changes, or the whole merge when a commit it brings in
  is unreviewed, are judged like any staged change. A merge that brings in
  more commits than routing's `gate.merge_commits` (50) is judged whole
  against HEAD, so the check stays within the gate's deadline. A merge after
  a command in the same line that moves HEAD (`git checkout`, `git switch`,
  `git commit` and the like) is blocked: run it as a command of its own. A
  pull is judged on the remote-tracking branch as it is before the pull
  fetches.
- **PR gate**: the feature review opens with `GATE: PASS` or
  `GATE: FAIL (<n> critical)`. It fails while an accepted critical finding,
  from this or an earlier feature review, has an open follow-up ticket. A
  gate holds for the head it reviewed: close each follow-up once its fix is
  on the feature branch, then `/flow finish` again.

[ADR-0002](../../docs/adr/0002-exactly-three-critique-hooks.md) allows three
hooks, for Claude Code and Codex only: `critique-commit-gate` (before
`git commit`, or a merge that records a merge commit), `critique-pr-gate` (before `gh pr create`, or a push outside
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
family, and for Vibe the model's `prices` in USD per million tokens:
`input`, `cached_input`, `output`, the same as its agent file's), each stage's base and deep rosters (client, role, lenses, price cap,
time limit), the `fallbacks`, the budget, the Antigravity quota, triage, the feature
branch patterns a push may update without a feature review (`gate`), the
stats thresholds and the repository packet's limits (`packet`: a 150 KB cap
in `cap_bytes`, 30 KB of fact files in `facts_cap_bytes`, a 30 KB
`tree/MANIFEST.tsv` in `manifest_cap_bytes`, a code packet's
20 KB repo map in `repo_map_cap_bytes`, 400-line windows in
`window_lines`, and the `exclude` globs for vendored, generated and lock
files; a file whose first lines say `@generated` or "do not edit" counts as
generated too). Model names live only there and in the
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
{ "antigravity_quota": { "daily_cap": 50 } }
```

Vibe's plan limit is not routing's: it is the model gateway's, in
`~/.config/agent-kit/model-gateway.json` under `clients.vibe.plan_limits`. An
override that still names the removed `budget.vibe_flow_cap_usd` is ignored,
and `setup` says so.

A default for every machine is a change to agent-kit's
`skills/workstation/critique/routing.json`.

**Decide what to change** with `/critique-stats`: from the ledger
(`~/.local/share/agent-kit/critique/ledger.jsonl`, metadata only) it reports
each critic client's precision, unique catches, fabrication, cost and
failures per stage. It suggests a swap only on 20 or more adjudicated
findings, flags a stage with no unique catch across its last three features,
and never edits routing.
