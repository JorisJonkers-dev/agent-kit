# Critique pipeline: expanding to ADRs, architecture and refactoring plans

Research notes, 2026-11-02. Not a decision: nothing here is settled until the
owner grills it and a spec records it. The words in bold in
[CONTEXT.md](../../CONTEXT.md) under "Critique pipeline" mean what they say
there. [integration.md](integration.md) describes how the pipeline works today.

## The question

How should the critique pipeline grow from reviewing one feature (decisions,
spec, tickets, diffs) to these four jobs?

1. **Review a whole project** and recover the decisions its code embodies
   as candidate ADRs.
2. **Review existing ADRs and architecture**: their quality, whether they
   agree with each other, and whether they agree with the code.
3. **Rewrite ADRs and write new ones.**
4. **Build refactoring plans** that improve the design and can feed
   `/to-spec` and `/to-tickets`.

The answer should rest on current practice: hexagonal architecture, DDD,
TDD, and the ADR and refactoring literature. It should also keep the
property that makes the pipeline worth having: a finding counts only when a
third party can check its evidence.

## How this was researched

- **Five Claude research passes.** These covered the existing skills and
  prompt packs, ADR practice, design principles turned into lenses,
  refactoring plans, and a code map of the runner's extension points. Every
  claim taken from them is cited below to its primary source.
- **A cross-family panel.** Mistral Large 4, GLM 5.3 and DeepSeek Flash 4.1
  each got the same context files, a focus question and the read-only
  critic agents. They answered from their own knowledge, without web
  access. Spend was $0.034, $0.077 and $0.025. The panel caught two things
  the Claude passes did not: the "archaeology trap" (§5.3) and the
  per-turn re-billing of packet size (§3.4). Two of GLM's citations were
  wrong and are not used here: *Learning DDD* is by Vlad Khononov, not
  Brandolini, and *Your Code as a Crime Scene* is by Adam Tornhill.

## 1. Findings in brief

1. **No popular pack has a lens that is checkable in our sense.** The most
   starred "architect" agents are lists of expertise, not checks:
   wshobson/agents `architect-review` (42k★), VoltAgent `architect-reviewer`
   (26k★) and the awesome-copilot SE architect. A typical item is
   "Scalability requirements met confirmed", which nothing can verify.
   Copying their prompts would bring in the taste the evidence rules exist
   to keep out. The usable material comes from the ADR literature (Olaf
   Zimmermann above all), Matt Pocock's gates and vocabulary, the DDD Crew
   canvases, and the orchestration mechanics of `ai-software-architect`.
2. **Design findings are checkable only against a rule the project itself
   declared.** "Hexagonal says so" is taste. A finding must cite either a
   rule the project declared (an ADR, a CONTEXT.md term, a layer list, an
   architecture-test config) and the line that breaks it, or a consequence
   that already shows: a use-case test that needs Docker, one change that
   touches N modules, two names for one concept. Without a declared rule,
   the same observation is an unasked question for the owner, not a defect.
3. **Critics cannot see a whole repository, so a script must.** Critics
   have `read_file` only. Import graphs, cycles, layer violations, write
   sites, churn, hotspots and co-change must be computed by `prepare` into
   the packet as settled facts. The critic interprets those facts and
   quotes them, and the runner verifies the quotes. The rule of thumb: the
   script finds where, and the model says whether it matters against a
   declared decision.
4. **Code shows what was decided, never why.** Recovered decisions are
   candidates for the owner, every one of them. A recovered ADR must never
   become evidence for later critiques until the owner accepts it.
   Otherwise the pipeline launders a model's guess into a constraint that
   the fidelity lens then defends.
5. **Repository-level reviews do not fit the feature flow.** The runner's
   flow model is strictly per feature (`FLOW_STAGES` = decisions, spec,
   tickets, feature-review). The new work needs a separate review track
   with its own entry points and spend key. Its output (accepted ADRs, a
   refactoring plan) then feeds the existing feature flow at `/to-spec`.
6. **Most of the machinery already exists.** Role and lens files, the
   quote check, `tree/` packets with `path:line` checks, the
   `check/tickets.json` pattern for script-computed settled facts, the
   needs-human ruling, the ledger and stats all carry over. The real gaps
   are four: a packet size cap (there is none today), a packet that holds
   both documents and code, a provenance flag for facts, and status-aware
   loading of decision docs.

## 2. What to borrow, and what to leave

| # | Item | Source | Becomes |
|---|---|---|---|
| 1 | ADR creation anti-patterns: Fairy Tale, Free Lunch Coupon, Dummy Alternative, Sprint, Sales Pitch, Problem-solution mismatch, Mega-ADR, Pseudo-accuracy | [Zimmermann 2023, ADR creation](https://www.ozimmer.ch/practices/2023/04/03/ADRCreation.html) | lens `adr-record` |
| 2 | The seven review questions, and the reviewer anti-patterns (Copy Edit, Self Promotion, Siding) | [Zimmermann 2023, ADR review](https://www.ozimmer.ch/practices/2023/04/05/ADRReview.html) | role `adr-reviewer`, and its scope rules |
| 3 | Definition of Ready (START) and Definition of Done (ecADR: Evidence, Criteria, Agreement, Documentation, Review plan) | [START](https://ozimmer.ch/practices/2023/12/01/ADDefinitionOfReady.html), [ecADR](https://www.ozimmer.ch/practices/2020/05/22/ADDefinitionOfDone.html) | gates for writing a new ADR |
| 4 | MADR 4's "Chosen option … because {driver}" and its *Confirmation* section | [MADR](https://adr.github.io/madr/) | lens checks: the rationale names a listed driver; the decision names the check that enforces it |
| 5 | Pocock's three-part ADR gate (hard to reverse, surprising without context, a real trade-off) and his "What qualifies" list | `mattpocock-skills/…/domain-modeling/ADR-FORMAT.md` | filter and search list for decision recovery |
| 6 | Seam heuristics: "One adapter means a hypothetical seam. Two adapters means a real one", the deletion test, "the interface is the test surface" | `mattpocock-skills/…/codebase-design/` | lens `test-seams`, and one bullet in `simplicity` |
| 7 | Bounded Context Canvas, Aggregate Design Canvas, the nine context-map patterns | [ddd-crew](https://github.com/ddd-crew) | lenses `domain-language` and `aggregates`; template for a recovered domain model |
| 8 | Domain red flags that an import or grep can verify: "Domain layer has zero infrastructure dependencies", "Database tables accessed by multiple contexts" | svngoku/coding-agents-skills `ddd/references/code-review.md` | lenses `dependency-direction` and `domain-language` |
| 9 | Challenge round: each critical finding goes to a member who did not raise it, who must answer NO OBJECTION, DOWNGRADE or REFUTE with a file:line | `ai-software-architect` 1.6.0, `architecture-review/SKILL.md` | optional refute step (phase 5) |
| 10 | Principle gate: a recommendation to change a principle never enters the fix list and becomes an ADR proposal instead | same | already our needs-human ruling; make it explicit for ADR conflicts |
| 11 | `motivated_by` provenance on each checklist item | same, `agents/*.md` | per-check provenance, so `/critique-stats` can prune single checks, not just whole lenses |
| 12 | Mechanical ADR validation (required sections, status enum) | same, `tools/lib/adr-validator.js` | runner pre-check `check/adrs.json` |
| 13 | Reader Testing: a no-context model answers questions about the document | [anthropics/skills doc-coauthoring](https://github.com/anthropics/skills/blob/main/skills/doc-coauthoring/SKILL.md) | role `cold-reader` for rewritten ADRs |
| 14 | Rejected refactor candidates with a load-bearing reason become ADRs, so later reviews stop re-suggesting them | `mattpocock-skills/…/improve-codebase-architecture` | ruling rule for refactoring plans |
| 15 | Expand–contract, prefactor-first ("make the change easy, then make the easy change") | `mattpocock-skills/…/to-tickets` | refactoring-plan step kinds |

Leave out:

- the architect persona prompts (capability lists);
- invented numbers, such as "Expected ROI: 280%" (wshobson `tech-debt`) and
  Necessity/Complexity 0–10 scores (Zimmermann's Pseudo-accuracy);
- "Strengths 3–5" praise quotas;
- invented "collaborative discussion" dialogue;
- the instruction to "explore organically" without citations
  (`improve-codebase-architecture`);
- any prompt that asks a model to say "why the current architectural
  pattern was chosen" from code alone (awesome-copilot
  `architecture-blueprint-generator`). This invites invented rationale.

## 3. Cross-cutting changes to the runner

The new stages share these changes, so they come first.

### 3.1 Evidence rules for design findings

These three rules go into one shared prompt fragment that the design stages
add after the preamble: `prompts/design-evidence.md`.

- **Declared rule or observed consequence.** A design finding cites either
  a rule the project declared and the place that breaks it, or a
  consequence visible in the packet. Without either, it is an
  `unasked-question` ("Is the billing domain meant to be independent of
  `pg`? `tree/src/billing/invoice.ts:3` imports it."). The orchestrator
  rejects anything else with `reason_code: taste`.
- **No verdict words without a threshold.** "Cohesive", "leaky", "anemic",
  "tightly coupled" and "clean" count only when tied to a number in
  `facts/` or a quoted decision. This comes from DeepSeek, on the panel.
- **A contradiction needs two quotes from two locations.** The quote check
  verifies that text exists, not that the inference is valid, so the
  inference has to be visible.

### 3.2 A `facts/` folder computed by `prepare`

Facts are deterministic. The same commit gives the same bytes. The time
window is counted back from the commit's own date, not from the wall clock.
Facts are listed in a `facts/INDEX.md`, because critics cannot list
directories. They are capped, and the prompt marks them settled, exactly as
`check/tickets.json` is marked for the ticket linter.

| File | Content | How it is computed |
|---|---|---|
| `INDEX.md` | what each file holds, its columns, the commit, the window, the thresholds, and every sampled file the budget dropped | runner |
| `churn.tsv` | per file: commits, lines added and deleted, author count (a count, never names), over 12 months | `git log --numstat` ([code-maat](https://github.com/adamtornhill/code-maat) format) |
| `hotspots.tsv` | churn × complexity, top 30 | indentation complexity ([Tornhill](https://pragprog.com/titles/atcrime2/your-code-as-a-crime-scene-second-edition/)), or `lizard` CCN when installed |
| `coupling.tsv` | co-changing file pairs, with a `crosses-module` flag | code-maat defaults: at least 5 revisions, at least 5 shared, degree at least 30%, changesets over 30 files skipped, test-to-subject pairs dropped |
| `imports.json` | module edges, cycles, fan-in and fan-out, instability `Ce/(Ca+Ce)` | dependency-cruiser or madge (TS/JS), import-linter or pydeps (Python), otherwise a regex import scan |
| `rules.json` | violations of layer rules, **only where the repository declares rules** | the repository's own `.dependency-cruiser.*`, `.importlinter` or ArchUnit config, in report mode |
| `writes.tsv` | the modules that write each table, file or collection | grep for write calls |
| `tests.tsv` | per module: source and test LOC, test files, tests that reach infrastructure, tests with no assertion | path conventions; coverage only from an existing report, never by running tests |
| `adrs.tsv` | each ADR's id, title, status, date, the paths it names, and its supersede links | parse `docs/adr/*.md` |
| `size.tsv` | the 30 largest tracked source files | `git ls-files` plus `wc -l`, excluding vendored and generated files |

**Facts go in a top-level `facts/` folder, not under `references/`.** The
runner records where each quote was found: a quote from an `artifact` file
counts as spec text, and a quote from a `reference` file counts as decision
text (`domain/evidence.ts`). An analysis file under `references/` would let
a churn number count as a recorded decision. Add a third provenance flag,
`fact`, beside `spec` and `decision`, so that a rule can require "a fact"
mechanically. A quote from a facts row then needs no new evidence kind.
(The refactoring pass proposed a `fact` kind and the lens pass a `metric`
kind. A provenance flag does the same job with less schema change.)

**Layer rules belong to the owner.** What counts as "domain" or "adapter"
must be declared once: in an architecture-test config, or in a CONTEXT.md
section that the runner renders into a temporary config. Otherwise the
script encodes the critic's taste, as GLM pointed out on the panel.

### 3.3 A packet that holds documents and code

Today a packet is either a document packet or a code packet, depending only
on `isCodeStage` (`domain/stage.ts`). ADR review against code, decision
recovery and architecture review all need both kinds in one packet:

- `artifact/` for the ADRs or plan under review;
- `references/` for the other ADRs and CONTEXT.md;
- `tree/` for the sampled code;
- `facts/`;
- optionally `history/`.

Changes needed:

- **Optional fields.** `CodePacketReader` with `base`, `spec` and `ticket`
  optional (`application/code-packet.ts:68-73`, `prepare.ts:277`).
- **Looser manifest.** `CodeSource` no longer requires `base_commit` and
  `changed_files`.
- **A curated file list** in place of "changed files"
  (`domain/manifest.ts:129-135`).
- **Code permission.** Each new stage joins `CODE_STAGES`, so the code
  permission and the `path:line` check apply. The runner already rejects
  a `tree/<path>:<line>` that does not exist at code stages
  (`domain/evidence.ts:95-112`); the ADR research proposed that check as
  new, but it is there.
- **An evidence rule per stage.** An unknown stage falls through to
  `ticketRule` today (`evidence.ts:127-131`), which demands a ticket id in
  the location.

### 3.4 A packet size cap

**Nothing caps the packet today.** `tree/` holds every non-binary blob at
the head commit, and each critic gets its own copy (`application/tripwire.ts:30-37`).
A whole-repository stage needs:

- **Exclusion globs** for vendored, generated and lock files.
- **A `tree/MANIFEST.tsv`**: every tracked path with its LOC and churn
  rank. Critics cannot list directories, so this is how they find files.
- **Full text for a ranked sample only**, in priority order:
  1. the files the artifact names;
  2. the top hotspots;
  3. each module's entry or interface file;
  4. the files on each import cycle.

  Each file is capped at about 400 lines and cut into windows that keep
  the original line numbers, so `tree/<path>:<line>` still resolves.
- **Drops by rule, never by a model.** Each drop is recorded in
  `facts/INDEX.md`, so a critic knows what it cannot see.

The refactoring pass proposed sizing the packet as
`0.6 × per-critic cap ÷ input price`. For Mistral Large at $1.40 per
million input tokens and the $0.50 cap, that is about 210k tokens. That
figure is too generous. A critic reads files in a tool loop, and every turn
re-sends the conversation so far, so the input billed grows with the square
of the number of reads. Cached-input prices soften this but do not remove
it. **Calibrate the cap on one real repository before routing the stage by
default**, as DeepSeek advised on the panel.

### 3.5 A separate review track

- **Entry points.** Each repository-level stage gets its own skill and its
  own flow key. Decisions already defaults its flow key to the run id
  (`prepare.ts:232`).
- **Spend key.** Each stage needs its own key in `budget-view.ts:45-47`.
  Otherwise it is charged against no flow, or the wrong one.
- **Outside the feature flow.** The new stages stay out of `FLOW_STAGES`.
  `secondRoundDue`, the report's "Do not start /to-tickets" line
  (`report.ts:192`) and `stats.ts:267` (which skips runs with no spec when
  looking for removal candidates) all assume a feature. Each needs a
  per-stage branch.
- **Antigravity quota.** The new stages are shut out of agy after 10 runs
  a day unless they join `antigravity_quota.reserved_stages`.
- **No hooks.** [ADR-0002](../adr/0002-exactly-three-critique-hooks.md)
  holds: exactly three hooks. The new stages are reached through their
  skills and `/flow`.
- **Glossary.** CONTEXT.md defines a **Critique stage** as "a point in the
  spec-to-ticket flow". Widen that definition, or add a term for
  review-track stages.

### 3.6 Status-aware decision docs

`DecisionDocs.resolve` (`application/decision-docs.ts`) loads every
`docs/adr/*.md` as decision evidence. Once recovered and proposed ADRs exist
on disk, it must skip any ADR whose status is `proposed`, `recovered`,
`rejected` or `superseded`. The house ADRs (0001, 0002) have no status line,
so **no status means accepted**. Only an explicit non-accepted status
excludes a record.

### 3.7 Categories

The sources disagree. The refactoring pass needed no new categories, the
ADR pass wanted three, and Mistral wanted four. Each new category changes
the schema and the output contract. That changes the prompt hashes, and
changed hashes reset the stats cells (`domain/prompt.ts:80-85`). So add
only categories whose verdict rule differs from every existing one:

| Category | Meaning | Why existing categories don't fit |
|---|---|---|
| `drift` | A decision record contradicted by a line of code | `contradiction` is between two documents, and `bug` presumes the spec is right. Here neither side is presumed right, so the verdict is always needs-human. |
| `stale` | A recorded assumption, version or revisit condition that the packet shows no longer holds | The record was right when it was written, so it is not an omission. |
| `undocumented-decision` | A candidate decision in code with no ADR (recovery only) | It is not a defect in an artifact, and it can never be accepted. |

Everything else maps onto existing categories:

- a missing alternative, cost or rationale is `omission`;
- vague or unconfirmable wording is `untestable`;
- two ADRs in force that disagree are a `contradiction`;
- missing supersede links and Mega-ADRs are `structure`;
- dependency, coupling and aggregate problems are `structure`;
- a decision with no fitness function is `untestable`. (The lens pass
  proposed an `unenforced` category; it is not needed.)

New reason codes for rejections: `taste`, `invented-rationale`,
`fact-misread`, `not-significant` (fails Pocock's gate) and
`duplicates-adr`.

## 4. Lens catalogue

Each of the 32 candidates was weighed on four things: whether a reader can
confirm the finding in one read, the false-positive risk, which artifacts
it applies to, and whether a script does the job better.

### New lenses (10)

| Lens | Checks | Sources | Used by |
|---|---|---|---|
| `adr-record` | One record at a time. Each check looks for content, never for a heading. The checks are listed below the table. | Nygard 2011, MADR 4, START/ecADR, Zimmermann's anti-patterns | adr-reviewer; the decisions stage when the artifact is a new ADR |
| `decision-log` | Records in force that contradict each other; a replacement with no link; a decision that names code but the code does the opposite (`drift`); a promised consequence missing from `tree/`; an assumption that no longer holds (`stale`); dangling references | Fowler, AWS, Azure, adr-tools; Su et al. 2026 | adr-reviewer |
| `dependency-direction` | An edge from inner to outer layer, with the importing line cited; a port declared in an adapter or named for a technology; database rows, HTTP or SDK types in domain signatures; business rules inside adapters | [Cockburn 2005](https://alistair.cockburn.us/hexagonal-architecture/), Cockburn & Garrido de Paz 2024, [Martin 2012](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html), Palermo 2008 | architecture-critic, plan-critic |
| `domain-language` | One term with two meanings, or two names for one concept, in docs and code; a domain word missing from the glossary; a context crossing with no named relationship (shared kernel, customer/supplier, conformist, ACL, published language); one store written by two contexts | Evans [DDD Reference](https://www.domainlanguage.com/wp-content/uploads/2016/05/DDD_Reference_2015-03.pdf), [ddd-crew context-mapping](https://github.com/ddd-crew/context-mapping), Bounded Context Canvas | architecture-critic, decision-recoverer |
| `aggregates` | An invariant enforced by a caller that another caller skips (with the input that breaks it); one transaction that writes two aggregates with no decision allowing it; a repository or setter that bypasses the aggregate root; "eventually" with no mechanism; a unit-bearing primitive mixed up at some call site | [Vernon 2011](https://www.dddcommunity.org/wp-content/uploads/files/pdf_articles/Vernon_2011_1.pdf), [Fowler, Anemic Domain Model](https://martinfowler.com/bliki/AnemicDomainModel.html), Aggregate Design Canvas | architecture-critic |
| `coupling` | A module depending on another's internals (file format, table layout, error text); one design decision encoded in several modules; co-change pairs across modules with no shared knowledge to explain them; pass-through methods; an interface as large as its implementation | Khononov, *Balancing Coupling* 2024 ([coupling.dev](https://coupling.dev/)); Ousterhout; Parnas 1972; [connascence.io](https://connascence.io/); Tornhill | architecture-critic, plan-critic |
| `test-seams` | A behaviour reachable only by a test that needs a real database, network or clock; tests coupled to internals; tests with no assertion, or asserting a value pasted from the output; a driven port with no fake held to the same behaviour | Cockburn; Feathers; Ian Cooper's "TDD, where did it all go wrong"; [Beck, Canon TDD 2023](https://newsletter.kentbeck.com/p/canon-tdd) | architecture-critic, plan-critic, feature-reviewer |
| `quality-scenarios` | A quality claim with no stimulus, environment or numeric response measure; a structural decision with no automated check (fitness function); quality goals that trade against each other with no stated priority; consequences that name only upsides | [SEI ATAM](https://insights.sei.cmu.edu/library/atam-method-for-architecture-evaluation/), ISO/IEC 25010:2023, *Building Evolutionary Architectures* 2e, [arc42 §10](https://docs.arc42.org/section-10/) | adr-reviewer, architecture-critic |
| `incremental-safety` | A step that changes structure and behaviour at once; a step on untested code with no characterize step before it; a big-bang change of an interface and all its callers; an order that leaves main broken (a trace); an irreversible step with no rollback note; a scaffold nothing removes | Beck, *Tidy First?*; Feathers; [Parallel Change](https://martinfowler.com/bliki/ParallelChange.html); [Branch by Abstraction](https://martinfowler.com/bliki/BranchByAbstraction.html); [Strangler Fig 2024](https://martinfowler.com/bliki/StranglerFigApplication.html) | plan-critic |
| `evidence-anchoring` | A step anchored to no fact row, ADR or prerequisite; a stale hotspot ordered ahead of real ones; a target that contradicts an accepted ADR; a target that closes an import cycle; splitting into services where an in-process seam would meet the goal; a port with only one adapter; a deletion that does not name the decision the deleted code implements (Chesterton's fence) | Tornhill/CodeScene; [MonolithFirst](https://martinfowler.com/bliki/MonolithFirst.html); Newman; DeepSeek's "do-nothing" check | plan-critic |

The `adr-record` checks, each looking for content rather than a heading:

- no sentence that states the decision;
- no alternative named, or no reason it lost;
- no cost or negative consequence;
- a context that already states the solution;
- no "because" tied to a quoted force;
- a claim about the outside world with no date or source;
- no way to confirm the decision, when one is possible;
- two choices in one record that could be reversed separately;
- vague wording, counted only when two readings lead to different code.

### Additions to existing lenses

- **`fidelity`**: a glossary `_Avoid_` term, or a synonym for a defined
  term, used in code or ADRs. Give the trace.
- **`testability`**: domain rules tested at a port with no database,
  network or framework.
- **`integration`**: a public interface (flag, format, schema, exported
  name) changed with no deprecation or version step.
- **`security`**: for each trust boundary, name the input that crosses it
  and the check applied ([STRIDE](https://learn.microsoft.com/en-us/azure/security/develop/threat-modeling-tool-threats)).
- **`failure-modes`**: an outbound call with no timeout or retry bound.
- **`simplicity`**: a port with one adapter and no test adapter (a
  hypothetical seam).
- **`operability`**: no change. It already covers observability and cost
  limits.

### Script checks, not model checks

- import cycles and package principles (ADP, SDP);
- layer violations against declared rules;
- framework imports in declared inner directories;
- write sites per store;
- tests that reach infrastructure, and tests with no assertion;
- a fitness-function inventory per ADR, reported as "none found", never
  as "missing";
- diagram edges (C4, Mermaid, Structurizr) against the import graph;
- 12-factor config reads, only where an ADR declares the rule;
- ADR structure (`check/adrs.json`): numbering, a status value from the
  enum when one is present, supersede and link targets that resolve, and
  superseded records still cited.

### Rejected

- **Test-shape ratios** (pyramid, trophy, honeycomb). Fowler calls the
  debate a distraction ([2021](https://martinfowler.com/articles/2021-test-shapes.html)).
- **Conway's law and team fit.** The critic cannot see the organisation;
  ask the owner during recovery instead.
- **SOLID by name, naming, "clean code".** Taste, and linter territory.
- **"Pattern X is missing".** It cannot be confirmed from the packet, and
  it invites the pattern-importing false positive.
- **Subdomain classification** (core, supporting, generic) as a lens. It
  is a business call, so it survives only as an unasked question.

## 5. The jobs, stage by stage

### 5.1 Review ADRs: `/critique-adrs` (stage `adr-review`)

- **Packet.**
  - `artifact/`: the ADRs under review (one, a set, or all).
  - `references/`: the other accepted ADRs and CONTEXT.md.
  - With the code permission on: `tree/` for the paths the ADRs name, plus
    `facts/adrs.tsv`.

  The decisions packet already supports several artifact files at repo
  paths (`decisions-packet.ts:53-58`).
- **Pre-check.** `check/adrs.json`, settled, before fan-out.
- **Role `adr-reviewer`.** It takes Zimmermann's stance as an official
  reviewer:
  - It never argues that the chosen option is wrong (the Self Promotion
    anti-pattern; matches gap-critic's "a decision already recorded stands
    as made").
  - It never copy-edits (Copy Edit; "style belongs to linters").
  - A record that fails Pocock's significance gate is at most `minor`.
- **Evidence.**
  - Every finding quotes `docs/adr/`.
  - A `contradiction` quotes two files.
  - `drift` and `stale` need a `tree/<path>:<line>` trace.
  - A vague-wording finding gives both readings and the code each one
    would produce.
- **Drift scope.** Drift checks are limited to decisions that name code,
  config or a command. On 980 ADRs from 109 repositories, LLM accuracy
  dropped for implicit and deployment-only decisions
  ([Su et al. 2026](https://arxiv.org/abs/2602.07609)).
- **Base routing (suggested).** Mistral Large with `adr-record`, and
  Gemini with `decision-log`. Deep adds GLM with `quality-scenarios` and
  `domain-language`.

Ruling:

| Finding | Verdict |
|---|---|
| Missing status or supersede link, where one record's text already says it replaces another | accepted (add the links, adr-tools style) |
| Broken reference, or a term out of line with CONTEXT.md where the meaning is unchanged | accepted |
| Missing alternative, rejection reason, cost or rationale | needs-human, unless the missing fact is quoted elsewhere in the packet; then accepted, citing it. The orchestrator never invents rationale. |
| Vague wording with two readings | needs-human: "which reading did you mean?" |
| Two records in force that contradict, with no replacing text | needs-human |
| `drift` | needs-human, always. Either the code is a bug (a ticket) or the ADR is stale (a superseding ADR). "Fix the code", AWS's default, is offered first, never applied. |
| No confirmation, where a mechanical check is possible | accepted as a follow-up ticket; the ADR text is unchanged |

### 5.2 Rewrite and write ADRs

**Rewriting.** The orchestrator may make these changes alone, because they
are editorial:

- add status and `Supersedes` / `Superseded by` / `Amends` links, where
  the packet already shows the relationship;
- fix broken references;
- align terms with CONTEXT.md where the meaning is unchanged;
- reorder sentences, putting the decision first (Fowler's "inverted
  pyramid");
- add cited facts the packet proves;
- split a Mega-ADR into records whose sentences already exist.

These never change in place: the chosen option, its scope and conditions,
the rationale, the rejected alternatives, and the stated consequences. A
change to any of them is a new ADR that supersedes the old one, or an
`Amends` ADR if it only narrows or extends the decision. Fowler, AWS, Azure
and adr-tools all agree on this. Every rewrite runs `adr-record` again, and
the diff may contain only changes from the allowed list.

Two checks follow a rewrite. The **cold reader** check: one cheap critic
with no context answers three questions from the rewritten ADR alone. What
was decided? What was rejected, and why? What would make us revisit it? A
wrong or unanswerable question is an `omission`. Separately, a check
compares the old and new text and flags any edit outside the allowed list.

**Writing a new ADR.** Pocock's `domain-modeling` already writes ADRs
during grilling, and the decisions stage already accepts decision docs as
its artifact. So the cheapest route is to add `adr-record` to the
decisions stage's roster whenever the artifact contains an ADR. No new
skill is needed.

Template choice:

- **Default: the house Pocock/Nygard paragraph.** Inline, it names at
  least one rejected alternative and one cost, so it passes `adr-record`
  and Pocock's own "real trade-off" test.
- **MADR 4** when there are three or more serious options, conflicting
  drivers that need a priority, parties beyond the owner, or a
  confirmation that can be automated. No weighted scoring tables
  (Pseudo-accuracy).
- **Y-statements** for recovered candidates, since one fits in a
  finding's `claim`.
- **From Tyree and Akerman, borrow only *Assumptions* and *Related
  requirements*.** The `stale` check tests the Assumptions.
- **START before drafting** (a missing letter becomes an owner question)
  and **ecADR after**, where "Agreement" means the owner.

### 5.3 Recover decisions from a project: `/recover-decisions` (stage `decision-recovery`)

The research is consistent on one point: code shows the consequences of a
decision, not its rationale. RecovAr recovered decisions from code plus
issue and commit history at about 75% recall and 77% precision on Hadoop
and Struts, and its authors say rationale "is usually missing or, at best,
implicit" in the structure ([Shahbazian et al.](https://arxiv.org/abs/1704.04798)).
LLMs produce relevant decisions "but not autonomously"
([Dhar et al., ICSA 2024](https://arxiv.org/abs/2403.01709)). Splitting the
work into extraction, retrieval, generation and validation agents beats one
prompt over a repository ([AgenticAKM 2026](https://arxiv.org/abs/2602.04445)).
Giving the model the 3–5 most recent ADRs as context helps
([Gupta et al., EASE 2026](https://arxiv.org/abs/2604.03826)).

- **Packet.**
  - `tree/` (sampled per §3.4);
  - `facts/`;
  - `history/log.txt`: `git log --stat` for the candidate paths;
  - `history/issues/` where the tracker is allowed;
  - every existing ADR;
  - CONTEXT.md.

  This needs the code permission.
- **Role `decision-recoverer`.** Pocock's "What qualifies" list is the
  search list. Each candidate is a finding in category
  `undocumented-decision`, with:
  - **a Y-statement in `claim`**, every clause marked *observed* or
    *inferred*;
  - **traces to at least two sites** (one site may be an accident);
  - **a quote from `history/` for every "because"**, or a plain statement
    that no rationale was found;
  - **evidence for each of Pocock's three tests**. "Real trade-off" means
    an alternative is visible somewhere: a removed dependency, a comment,
    an issue;
  - **a confidence**: high when code, config and history agree; low when
    the evidence is code only, or the decision is implicit or
    deployment-only;
  - **the draft ADR in `suggestion`**, in house format with
    `Status: recovered`.
- **Ruling.** A recovered decision is never accepted. The ecADR
  "Agreement" criterion needs a human, and AWS reserves acceptance for the
  owner.
  - **Rejected mechanically:** a candidate with no trace, only one site,
    an invented rationale, a failed significance test, or a duplicate of
    an existing ADR. A duplicate belongs to `decision-log` drift instead.
  - **Needs-human otherwise**, one candidate at a time: "Is this a
    decision you made, and why? What did you reject?" The ADR is written
    from the owner's answer, keeping the observed facts and replacing the
    inferred rationale with the owner's words.
  - **If the owner says it was an accident**, the candidate becomes input
    to a refactoring plan, not an ADR.
- **The archaeology trap.** DeepSeek on the panel called this "the
  highest-value single rule". A `recovered` ADR is never loaded as
  decision evidence (§3.6) until the owner accepts it.

### 5.4 Review the architecture: `/review-architecture` (stage `architecture-review`)

- **Packet.** `facts/`, sampled `tree/`, all accepted ADRs, CONTEXT.md, the
  README and build configs.
- **Role `architecture-critic`.** It reviews as the engineer who will
  change this code next quarter. It looks for breaks of declared rules
  and for consequences already visible, under the evidence rules in §3.1.
  Every finding names the observable that would show a fix worked: a new
  architecture-test rule, a test that no longer needs Docker, co-change
  dropping. That observable later becomes the plan step's "Done when" and
  the ticket's acceptance criterion.
- **Base routing (suggested).**
  - Gemini with `dependency-direction` and `coupling`.
  - Mistral Large with `domain-language` and `aggregates`.
  - Deep adds GLM with `test-seams` and `quality-scenarios`.
  - Deep also adds DeepSeek as `decision-recoverer`, so a review also
    yields ADR candidates.
- **Ruling.**
  - **Accepted** when the finding breaks a declared rule, and the fix is a
    ticket that restores that rule.
  - **Needs-human** when the fix would change a rule or a target.
  - **Rejected** (`taste`) when neither a declared rule nor a visible
    consequence backs it.
- **Output.** A ranked list of accepted findings. This is the input to
  `/improve-codebase-architecture`, which already grills candidates with
  the owner. The improvement over today: every candidate card arrives with
  cited lines and facts, instead of "explore organically".

### 5.5 Build and critique a refactoring plan: `/critique-plan` (stage `plan`)

This stage sits after the grilling loop of `/improve-codebase-architecture`
and before `/to-spec`.

**Artifact: `refactoring-plan.md`.** Its sections map one-to-one onto the
`/to-spec` template:

| Plan section | `/to-spec` section |
|---|---|
| Goal (the outcome it unblocks, with evidence as fact anchors) | Problem Statement and Solution |
| Target (modules and interfaces in glossary terms; seams; dependency rules; ADRs relied on, superseded, or needed) | Implementation Decisions |
| Safety net | Testing Decisions |
| Not doing | Out of Scope |

Two further sections:

- **Steps.** One ticket per step; the `Blocked by` edges become the
  ticket graph.
- **Open decisions.** Must be empty before `/to-spec`.

Each step has these fields:

- **Kind**: exactly one of characterize, tidy, structure, behaviour,
  expand, migrate, contract, remove-transitional;
- **Technique**: a catalog name such as Extract Function, or "Branch by
  Abstraction step 2";
- **Anchor**: a fact row, an ADR, or `prerequisite-of:Sn`;
- **Touches**;
- **Safety net**;
- **Main stays green because**;
- **Rollback**;
- **Transitional element** (and the step that removes it);
- **Blocked by**;
- **Done when**.

The format enforces these rules, and `check/plan.json` verifies them
deterministically:

- one Kind per step;
- `behaviour` steps only when the Goal says behaviour changes (Beck: tidy
  first, in its own commits);
- structure work on untested code is blocked by a characterize step
  (Feathers);
- every expand has a migrate and a contract (Sato), and every toggle or
  shim has a removal step (Hodgson; Cartwright et al.);
- the steps form a DAG and are worked from the leaves (Mikado);
- each step fits one session and ships alone;
- file paths appear only inside fact anchors, because `/to-spec` and
  `/to-tickets` forbid paths.

**Role `plan-critic`**, with lenses `incremental-safety` and
`evidence-anchoring`. Each finding breaks exactly one named rule. Its
location reads like `S03: mixed-change`. Severity comes from the rule:
`no-rollback` on a data step is critical, for example. The critic does not
choose it. This idea comes from DeepSeek on the panel: it makes severity
calibration measurable in `/critique-stats`. Base routing (suggested):
DeepSeek Flash with `incremental-safety`, and Mistral Large with
`evidence-anchoring`.

**Ruling.**

- **Accepted** when the fix changes only how, or in what order, the plan
  reaches its Target: split, reorder, add a characterize, contract,
  rollback or removal step, move cold-code tidying to "Not doing".
- **Needs-human** when the fix changes the Target itself (modules,
  interfaces, seams, dependency rules), an ADR's standing, or accepts an
  irreversible step or a permanent dual path.
- **A clash with an accepted ADR** is one question: "Keep ADR-NNNN and
  drop S0n, or supersede ADR-NNNN?" Choosing supersede hands over to the
  ADR-writing flow before `/to-spec`.
- **Rejected:** `taste` (a different target, or a pattern by name) and
  `fact-misread`.
- **A rejected candidate with a load-bearing reason becomes an ADR**, so
  later reviews stop re-suggesting it.

## 6. Risks, and the rule that answers each

| Risk | Answer |
|---|---|
| Fluent, invented rationale | `invented-rationale` rejection; every "because" needs a history quote; recovered ADRs are never evidence (§5.3) |
| Pattern-importing false positives ("not hexagonal") | the declared-rule-or-consequence rule; rejection with `taste` (§3.1) |
| Accurate quotes followed by a non-sequitur | two quotes for a contradiction; numbers come only from `facts/`; verdict words need a threshold |
| Code treated as authority over a decision, or the reverse | `drift` is always needs-human (§5.1) |
| Critics counting or estimating, which models do badly | facts computed by the script and marked settled (§3.2) |
| Cost growing with repository size | sampled `tree/` with a cap and recorded drops; calibrate before default routing (§3.4) |
| Cheap plausible findings beating careful empty reviews | score precision (accepted ÷ reported) per role and per check in stats; count a well-argued `no_findings_reason` as success |
| The orchestrator grading its own homework | the ledger keeps every verdict; the owner audits a random 10% of rejections (panel suggestion) |
| Implicit and deployment decisions misjudged | drift limited to decisions that name code; such candidates are marked low confidence (Su et al. 2026) |

## 7. Suggested phasing

Each phase is shippable on its own and is measured with `/critique-stats`
before the next one starts.

0. **Runner groundwork.**
   - a packet size cap, exclusion globs and `tree/MANIFEST.tsv`;
   - a packet that holds both documents and code;
   - a `fact` provenance flag;
   - status-aware decision docs;
   - repository-level flow and spend keys;
   - categories `drift`, `stale` and `undocumented-decision`, and the new
     reason codes;
   - a new CONTEXT.md glossary entry;
   - an ADR for the review track.
1. **ADR review.**
   - `check/adrs.json`, `adr-reviewer`, and the lenses `adr-record` and
     `decision-log`, documents only;
   - `/critique-adrs`;
   - `adr-record` added to the decisions stage when its artifact is an
     ADR;
   - the rewrite rules and the cold-reader check.

   This phase is the cheapest, has the highest value, and needs no code
   permission. agent-kit's own ADRs are the first dogfood run.
2. **Facts and drift.** The facts script; `decision-log` drift and stale
   checks against `tree/`; `/recover-decisions` with the history packet.
3. **Architecture review.** The six design lenses, `architecture-critic`,
   `/review-architecture`, and the output handed to
   `/improve-codebase-architecture`.
4. **Refactoring plans.** The plan format, `check/plan.json`,
   `plan-critic`, and `/critique-plan`, wired between
   `/improve-codebase-architecture` and `/to-spec`.
5. **Calibration.**
   - severity derived from rules;
   - per-check provenance (`motivated_by`) and per-check precision in
     stats;
   - the 10% rejection audit;
   - an optional refute round for accepted criticals, on a family that
     did not raise them;
   - a self-reported confidence field was considered and deferred. Models
     overrate their own output
     ([Panickssery et al. 2024](https://arxiv.org/abs/2404.13076)), and
     agreement across families on one location already merges findings.

## 8. Questions for the owner (answered in §9)

1. **One skill per stage, or one skill with modes?** The options are
   `/critique-adrs`, `/recover-decisions`, `/review-architecture` and
   `/critique-plan` as separate skills, or one `/review-architecture` with
   modes.
2. **Add the three categories**, which resets stats cells for every stage,
   or carry them as reason codes under existing categories?
3. **Where do recovered and proposed ADRs live?** In `docs/adr/` with
   `Status: recovered`, or outside the decision docs (for example
   `.scratch/`) until accepted?
4. **Should the house ADR format require one rejected alternative and one
   cost inline?** That is Pocock's own third test, made explicit. The
   requirement can live in our lens without changing the upstream skill.
5. **Where are layer rules declared?** In a dependency-cruiser config or
   in a CONTEXT.md section. Without them, `dependency-direction` produces
   only questions.
6. **May commit subjects and issue text go to other families?** Recovery
   needs history quotes for rationale, but the refactoring pass proposed
   leaving commit messages out of `facts/` for privacy. This needs its own
   switch beside the code permission.
7. **Budget.** Keep $0.50 per critic and $5 per flow for repository-level
   stages? Should `architecture-review` join the Antigravity reserved
   stages?
8. **Where does a refactoring plan live?** As a tracker issue like a spec,
   or as a local file until `/to-spec`?

## Sources not verified first-hand

- ISO/IEC/IEEE 42010:2022: the text is paywalled; this rests on a summary
  and the table of contents.
- Zimmermann's Y-statement post returned HTTP 403; the template came from
  adr.github.io and the Design Practice Repository.
- The wording of Mikado's revert step and Feathers' change algorithm: the
  books were not read; the claims rest on the publishers' pages.
- These were cited from the researchers' own knowledge, not re-fetched:
  Vernon parts 2–3, Ousterhout, the details of Khononov's book, and
  ISO 25010:2023's characteristic names.
- The three panel answers were model knowledge without web access. Only
  their ideas are used here, never their citations.

## 9. Owner decisions (grilling, 2026-11-02)

The owner answered the questions in §8 and the ones they opened. Where this
section and the sections above disagree, this section wins. New terms are in
[CONTEXT.md](../../CONTEXT.md) (**Review stage**, **Fact**, **Recovered
decision**, **Refactoring plan**, **Refute round**). The decisions that pass
the ADR gate are [ADR-0003](../adr/0003-review-stages-run-outside-the-feature-flow.md),
[ADR-0004](../adr/0004-critics-never-compute-facts.md) and
[ADR-0005](../adr/0005-refactoring-plans-are-their-own-critique-stage.md).

### Scope and delivery

1. **One spec covers all phases 0–5.** Its tickets are grouped by phase. Each
   phase ends with a ticket for a dogfood live check on agent-kit, and that
   ticket blocks the next phase's tickets.

### Shape

2. **Each stage gets its own skill.** These are `/critique-adrs`,
   `/recover-decisions`, `/review-architecture` and `/critique-plan`.
3. **ADR review, decision recovery and architecture review are review
   stages.** They run outside the feature flow, and each run is its own flow
   keyed by its run id (ADR-0003).
4. **The plan is a critique stage in the feature flow** (ADR-0005).
   - **Where it sits.** For a refactor it replaces the decisions stage:
     `/improve-codebase-architecture` grilling, then plan, then `/to-spec`.
   - **What marks a refactor.** A draft is a refactor when a plan file exists
     for it.
   - **Who writes the plan.** `/critique-plan` drafts the plan when none
     exists.
   - **Where the plan file lives.** It follows the repository's tracker
     convention. With a local-markdown tracker it goes under
     `.scratch/<feature>/`. Otherwise it sits beside the flow state in the
     git common directory and is never committed.
   - **Gate.** `/to-spec` runs only when the plan's Open decisions section
     is empty.
5. **The plan format is accepted as written in §5.5.** `check/plan.json`
   enforces it.
6. **No new skill for writing ADRs.**
   - Grilling writes them.
   - The decisions stage adds an `adr-record` critic when its artifact
     holds an ADR.
   - `/critique-adrs` and `/recover-decisions` draft ADRs only from the
     owner's answers.

### Findings and evidence

7. **Three new categories, all added in phase 0:** `drift`, `stale` and
   `undocumented-decision`. This resets the stats once.
8. **Every lens bullet gets a stable check id, old lenses and new.**
   Findings name the check they come from, and stats report precision per
   check. This reset ships in the same release as the category reset.
9. **Checklist-style findings have a fixed severity per rule.** This covers
   the `adr-record` checks and the plan-critic rules. The critic cannot
   change the severity. The orchestrator can, with a reason.
10. **The design-evidence rules in §3.1 are accepted, all three:**
    - a declared rule or a visible consequence;
    - verdict words only with a threshold;
    - two quotes for a contradiction.
11. **The six additions to existing lenses in §4 are accepted.**
12. **`adr-record` requires at least one rejected alternative with its
    reason, and at least one cost.** It looks for them in the record's text,
    not under headings. This is in our lens only; Pocock's format stays
    unchanged.
13. **Drift checks apply only to decisions that name code, config or a
    command.** Recovery marks implicit and deployment-only candidates as low
    confidence.

### Decision records

14. **An ADR with no status line counts as accepted.** Only an explicit
    `proposed`, `rejected`, `superseded` or `deprecated` status removes a
    record from decision evidence.
15. **`/critique-adrs` reviews every ADR by default.** An argument narrows
    it to the named ADRs, and the rest become references.
16. **Allowed in-place rewrites.**
    - **The list.** The editorial edits in §5.2 may be made without the
      owner. Everything else needs a new superseding or amending ADR.
    - **How each edit is checked.** The orchestrator labels every edit with
      its kind from the allowed list. `adr-record` then runs again, and the
      cold reader checks the result.
    - **What the runner checks.** It confirms that the decision sentence is
      byte-identical.
17. **The cold reader runs on Mistral Small, capped at $0.03.**
18. **Drift answers.**
    - **"The code is wrong"**: the orchestrator files a tracker issue
      (`type:bug`, linking the ADR) once the owner agrees.
    - **"The ADR is stale"**: the orchestrator drafts a superseding ADR. The
      owner approves its text, and the old record is marked superseded.

### Recovery

19. **Candidates live only in the run directory and the report** until the
    owner accepts them. They are then written to `docs/adr/` in house
    format, with no status line.
20. **Recovery runs standalone, and also inside a deep architecture
    review.**
21. **At most 10 owner questions per run**, in order of confidence. The rest
    are listed in the report as deferred.
22. **Mechanical rejection.** A candidate is rejected when any of these
    holds:
    - it has no `tree/` trace;
    - it has only one site;
    - a "because" has no history quote and no "none found";
    - it fails one of Pocock's tests;
    - it duplicates an existing ADR.
23. **After the owner accepts ADRs, the skill suggests `/critique-adrs`.**
    It spends nothing automatically.
24. **History needs a new `history` permission** in the family allowlist,
    beside `code` and off by default. History means commit subjects and
    bodies, and issue text. Without the permission, every rationale reads
    "none found".

### Facts, packets and architecture review

25. **How facts are computed** (ADR-0004).
    - **Always.** Built-in git scans and a regex import scan.
    - **Layer violations.** These come only from the repository's own
      architecture-test config, run in report mode when its binary is
      already installed.
    - **Layer rules.** The config is the only source. A repository with no
      config gets only unasked questions from `dependency-direction`.
26. **Facts need the code permission.** Any stage that sends facts or
    `tree/` is a code stage.
27. **Packet limits.**
    - **The cap.** Repository-level packets start at 150 KB, with files cut
      into 400-line windows. `facts/` is at most 30 KB and is always
      included.
    - **Calibration.** The phase-0 live check sets the final numbers in
      routing.
28. **Budget is unchanged.**
    - **Caps.** $0.50 per critic and $5 per flow.
    - **Antigravity quota.** The new stages are not added to the Antigravity
      reserved stages.
29. **`/review-architecture`'s accepted findings become cited candidates in
    its report.** `/flow` suggests `/improve-codebase-architecture` on the
    one the owner picks. No issues are filed automatically.

### Routing and rounds

30. **Base rosters are as suggested in §5.** Deep rosters are in brackets.
    - **`adr-review`**: Mistral Large with `adr-record`, Gemini with
      `decision-log` [GLM with `quality-scenarios` and `domain-language`].
    - **`architecture-review`**: Gemini with `dependency-direction` and
      `coupling`, Mistral Large with `domain-language` and `aggregates`
      [GLM with `test-seams` and `quality-scenarios`, DeepSeek as
      `decision-recoverer`].
    - **`decision-recovery`**: DeepSeek and Mistral Large, both as
      `decision-recoverer`.
    - **`plan`**: DeepSeek with `incremental-safety`, Mistral Large with
      `evidence-anchoring` [GLM with `coupling` and `test-seams`].
31. **Depth defaults.** Deep runs when any of these fires.
    - **`adr-review`**: more than 10 ADRs in the artifact, or a risk term.
    - **`decision-recovery` and `architecture-review`**: more than 20k
      tracked source lines, or a `CONTEXT-MAP.md`.
    - **`plan`**: more than 8 steps, or a migrate or contract step that
      touches stored data.
32. **Only `adr-review` and `plan` get a second round.** These are the two
    stages that revise their artifact.
33. **The refute round runs at every stage.**
    - **What goes in.** Each accepted critical finding, after the verdicts
      and before fixes and owner questions.
    - **Who refutes.** The first routed client, base before deep, whose
      family neither raised the finding nor is the orchestrator's. With no
      such client, the round is skipped and the skip is noted.
    - **Effect on the second round.** The second-round count uses the
      criticals still accepted after the refute round.
34. **Calibration in scope:** per-check precision (decision 8) and the
    refute round. **Out of scope:** the 10% audit of rejected findings, and a
    self-reported confidence field.

### From `/critique-decisions` (run 20261102T064601Z-decisions)

35. **A failed decision-sentence check undoes the rewrite.** When the runner
    finds that a rewrite changed an ADR's decision sentence, it refuses to
    record. The orchestrator restores that ADR's original text and asks the
    owner whether the change is a new decision, which needs a superseding
    ADR, or a mistake.
36. **The new stages reuse the spec stage's `risk_terms` list for depth.**
    These are `adr-review`, `decision-recovery`, `architecture-review` and
    `plan`.
37. **A cold-reader miss undoes the rewrite.** If the cold reader cannot
    answer what was decided, what was rejected and why, or when to revisit,
    that is an omission. The orchestrator restores the original text of that
    ADR and lists it in the report.
38. **What the decision-sentence check verifies when a Mega-ADR is split.**
    - Every original decision sentence appears byte-identical in exactly one
      of the new files.
    - No new file adds a decision sentence.
    - The original record is marked superseded by the split files.
39. **With the code permission off, `/critique-adrs` sends only ADRs and
    CONTEXT.md.** It skips the drift, stale-against-code and confirmation
    checks, and its report names each skipped check and why.
40. **Accepting a recovered decision asks four things.** "Is this your
    decision?", "Why?", "What did you reject?" and "What does it cost?" The
    ADR is written in Pocock's house format with the answers in its text, so
    it passes `adr-record`.
41. **The cold reader runs through the existing `vibe-mistral-small`
    client.** It is family `mistral`, capped at $0.03 per check, and its
    spend counts in the review run's $5 flow cap.
42. **The owner starts the first `/critique-plan` of a refactor.** `/flow`
    may suggest it after `/improve-codebase-architecture`. The plan file
    that run drafts then marks the draft as a refactor, and `/flow` tracks
    it from there.
