import type { Ledger, LedgerCriticRun, LedgerFinding, LedgerRunContext } from './ledger.js'
import { sha256 } from './manifest.js'
import { SEVERITIES, type Severity } from './findings.js'
import { CritiqueError } from './outcomes.js'
import type { StatsThresholds } from './routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from './stage.js'
import { STAGE_TABLE, type RemovalRule } from './stage-table.js'

/** Which ledger lines the figures cover. Dates are `YYYY-MM-DD`, both ends included, read from `recorded_at` in UTC. */
export interface StatsFilter {
  /** The allowlist key, `host/owner/repo`; compared ignoring case. */
  readonly repository?: string
  readonly since?: string
  readonly until?: string
}

/** Accepted over adjudicated for one slice of a cell's findings. */
export interface PrecisionSlice {
  /** Accepted plus rejected findings in the slice. */
  readonly adjudicated: number
  readonly accepted: number
  /** Null with nothing adjudicated in the slice. */
  readonly precision: number | null
}

/**
 * Whether a critic is right over time and whether its severities mean what
 * they say. Only a cell with the routing's minimum of adjudicated findings
 * has one.
 */
export interface StatsCalibration {
  /** The older half of the cell's adjudicated findings, ordered by when their run was recorded. */
  readonly earlier: PrecisionSlice
  /** The newer half; it holds the odd finding. */
  readonly recent: PrecisionSlice
  /** Recent precision minus earlier precision; null when either half has none. */
  readonly drift: number | null
  /** Precision per severity the finding was recorded with. A critic is calibrated when its criticals are right at least as often as its minors. */
  readonly by_severity: Readonly<Record<Severity, PrecisionSlice>>
}

/**
 * One cell: one critic client on one stage with one prompt. A changed role,
 * lens, preamble, contract or schema file changes the prompt hash, so a
 * revised prompt is compared on its own findings, never mixed with the old.
 */
export interface StatsCell {
  readonly stage: CritiqueStage
  readonly client: string
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
  /** The first twelve hex digits of a hash over every prompt file hash the critic's runs carry. */
  readonly prompt_hash: string
  readonly first_recorded_at: string
  readonly last_recorded_at: string
  readonly runs: number
  /** Runs that gave no usable review: failed or violated read-only. */
  readonly failed_runs: number
  /** Failed runs stopped by a time limit. Vibe's exit 3 is a deadline or a price-limit stop, and both count. */
  readonly timed_out_runs: number
  /** Every finding a critic of this cell raised, alone or with others. */
  readonly findings: number
  readonly accepted: number
  readonly rejected: number
  readonly hints: number
  readonly needs_human: number
  /** Accepted plus rejected: the findings precision is measured on. */
  readonly adjudicated: number
  /** True when the cell has too few adjudicated findings for a suggestion. */
  readonly below_threshold: boolean
  /** Accepted findings no other critic client raised. */
  readonly unique_catches: number
  /** Findings fan-out rejected because a quote is in no packet file. */
  readonly fabricated: number
  /** Accepted over adjudicated; hints and needs-human are left out. Null with nothing adjudicated. */
  readonly precision: number | null
  /** Fabricated over findings; null with no findings. */
  readonly fabrication_rate: number | null
  readonly failure_rate: number
  readonly timeout_rate: number
  /** The summed cost of the runs whose cost is known. */
  readonly cost_usd: number
  /** Runs with no known cost (Antigravity is quota, not money; a deadline stop reports none). */
  readonly unpriced_runs: number
  /** Cost over accepted findings; null with no accepted finding or no priced run. */
  readonly cost_per_accepted_usd: number | null
  /** Runs a price or time limit stopped (Vibe's `price_limit`, its deadline, or an interrupted run) before they found anything. */
  readonly stopped_runs: number
  /** The summed known cost of the stopped runs: spend that bought no finding. */
  readonly wasted_usd: number
  /** Null while the cell is below the adjudicated-findings threshold. */
  readonly calibration: StatsCalibration | null
}

/**
 * A stage whose latest features, or at a review stage whose latest runs,
 * brought no accepted finding that only one critic client raised.
 */
export type RemovalCandidate =
  | {
      readonly stage: CritiqueStage
      /** The features (spec references) looked at, latest first. */
      readonly features: readonly string[]
    }
  | {
      readonly stage: CritiqueStage
      /** The review runs looked at, latest first. */
      readonly runs: readonly string[]
    }

export interface CritiqueStats {
  readonly cells: readonly StatsCell[]
  readonly removal_candidates: readonly RemovalCandidate[]
  /** Plain sentences for the owner. Nothing applies them. */
  readonly suggestions: readonly string[]
}

const TIMEOUT_FAILURES: ReadonlySet<string> = new Set([
  'print-timeout',
  'stopped-early',
  'exit-124',
])

/** A limit stopped the run: a time or price limit, or the outer timer that killed it. */
const STOPPED_FAILURES: ReadonlySet<string> = new Set([...TIMEOUT_FAILURES, 'timeout'])

/** One adjudicated finding of a cell, as calibration reads it. */
interface JudgedFinding {
  readonly accepted: boolean
  readonly at: string
  readonly severity: Severity
}

function slice(findings: readonly JudgedFinding[]): PrecisionSlice {
  const accepted = findings.filter((finding) => finding.accepted).length
  return {
    accepted,
    adjudicated: findings.length,
    precision: findings.length === 0 ? null : accepted / findings.length,
  }
}

/** The cell's findings in the order their runs were recorded, split in two halves, and by severity. */
function calibrationOf(judged: readonly JudgedFinding[]): StatsCalibration {
  const ordered = [...judged].sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0))
  const half = Math.floor(ordered.length / 2)
  const earlier = slice(ordered.slice(0, half))
  const recent = slice(ordered.slice(half))
  return {
    by_severity: Object.fromEntries(
      SEVERITIES.map((severity) => [severity, slice(ordered.filter((f) => f.severity === severity))]),
    ) as Record<Severity, PrecisionSlice>,
    drift:
      earlier.precision === null || recent.precision === null
        ? null
        : recent.precision - earlier.precision,
    earlier,
    recent,
  }
}

/** One cell while the ledger is read. */
class Tally {
  readonly stage: CritiqueStage
  readonly client: string
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
  first = ''
  last = ''
  runs = 0
  failed = 0
  timedOut = 0
  findings = 0
  accepted = 0
  rejected = 0
  hints = 0
  needsHuman = 0
  unique = 0
  fabricated = 0
  cost = 0
  unpriced = 0
  stopped = 0
  wasted = 0
  /** Every accepted or rejected finding of the cell, for calibration. */
  readonly judged: JudgedFinding[] = []

  constructor(
    run: LedgerCriticRun,
    readonly promptHash: string,
  ) {
    this.stage = run.stage
    this.client = run.client
    this.model = run.model
    this.role = run.role
    this.lenses = run.lenses
  }

  addRun(run: LedgerCriticRun): void {
    this.runs += 1
    if (this.first === '' || run.recorded_at < this.first) this.first = run.recorded_at
    if (run.recorded_at > this.last) this.last = run.recorded_at
    if (run.status !== 'ok') this.failed += 1
    if (run.failure !== null && TIMEOUT_FAILURES.has(run.failure)) this.timedOut += 1
    if (run.cost_usd === null) this.unpriced += 1
    else this.cost += run.cost_usd
    if (run.failure !== null && STOPPED_FAILURES.has(run.failure) && run.finding_count === 0) {
      this.stopped += 1
      this.wasted += run.cost_usd ?? 0
    }
  }

  addFinding(finding: LedgerFinding, unique: boolean): void {
    this.findings += 1
    if (finding.verdict === 'accepted') this.accepted += 1
    if (finding.verdict === 'rejected') this.rejected += 1
    if (finding.verdict === 'hint') this.hints += 1
    if (finding.verdict === 'needs-human') this.needsHuman += 1
    if (finding.verdict === 'accepted' && unique) this.unique += 1
    if (finding.pre_verdict === 'fabricated-evidence') this.fabricated += 1
    if (finding.verdict === 'accepted' || finding.verdict === 'rejected')
      this.judged.push({
        accepted: finding.verdict === 'accepted',
        at: finding.recorded_at,
        severity: finding.severity,
      })
  }

  cell(thresholds: StatsThresholds): StatsCell {
    const adjudicated = this.accepted + this.rejected
    const priced = this.runs - this.unpriced
    const belowThreshold = adjudicated < thresholds.min_adjudicated_findings
    return {
      accepted: this.accepted,
      adjudicated,
      below_threshold: belowThreshold,
      calibration: belowThreshold ? null : calibrationOf(this.judged),
      client: this.client,
      cost_per_accepted_usd: this.accepted === 0 || priced === 0 ? null : this.cost / this.accepted,
      cost_usd: this.cost,
      fabricated: this.fabricated,
      fabrication_rate: this.findings === 0 ? null : this.fabricated / this.findings,
      failed_runs: this.failed,
      failure_rate: this.failed / this.runs,
      findings: this.findings,
      first_recorded_at: this.first,
      hints: this.hints,
      last_recorded_at: this.last,
      lenses: this.lenses,
      model: this.model,
      needs_human: this.needsHuman,
      precision: adjudicated === 0 ? null : this.accepted / adjudicated,
      prompt_hash: this.promptHash,
      rejected: this.rejected,
      role: this.role,
      runs: this.runs,
      stage: this.stage,
      stopped_runs: this.stopped,
      timed_out_runs: this.timedOut,
      timeout_rate: this.timedOut / this.runs,
      unique_catches: this.unique,
      unpriced_runs: this.unpriced,
      wasted_usd: this.wasted,
    }
  }
}

/**
 * The figures per cell, the stages that are candidates for removal, and the
 * routing suggestions, from the ledger lines the filter keeps. `routed`
 * names the clients a stage's base roster routes to now. A finding counts
 * for every cell whose critic raised it.
 */
export function critiqueStats(
  ledger: Ledger,
  filter: StatsFilter,
  thresholds: StatsThresholds,
  routed: (stage: CritiqueStage) => readonly string[],
): CritiqueStats {
  const runs = ledger.runs.filter((line) => kept(line, filter))
  const tallies = new Map<string, Tally>()
  const byCritic = new Map<string, Tally>()
  for (const run of runs) {
    const hash = promptHash(run)
    const key = `${run.stage} ${run.client} ${hash}`
    const tally = tallies.get(key) ?? new Tally(run, hash)
    tallies.set(key, tally)
    tally.addRun(run)
    byCritic.set(criticKey(run.run_id, run.critic_id), tally)
  }
  const uniqueCatches: LedgerFinding[] = []
  for (const finding of ledger.findings.filter((line) => kept(line, filter))) {
    const raisers = new Set(finding.raised_by.map((id) => raiser(byCritic, finding, id)))
    const unique = new Set([...raisers].map((tally) => tally.client)).size === 1
    if (unique && finding.verdict === 'accepted') uniqueCatches.push(finding)
    for (const tally of raisers) tally.addFinding(finding, unique)
  }
  const cells = [...tallies.values()]
    .map((tally) => tally.cell(thresholds))
    .sort(
      (a, b) =>
        CRITIQUE_STAGES.indexOf(a.stage) - CRITIQUE_STAGES.indexOf(b.stage) ||
        a.client.localeCompare(b.client) ||
        a.first_recorded_at.localeCompare(b.first_recorded_at),
    )
  return {
    cells,
    removal_candidates: removalCandidates(runs, uniqueCatches, thresholds.removal_features),
    suggestions: suggestions(cells, routed),
  }
}

function kept(line: LedgerRunContext, filter: StatsFilter): boolean {
  const day = line.recorded_at.slice(0, 10)
  return (
    (filter.repository === undefined ||
      line.repository.toLowerCase() === filter.repository.toLowerCase()) &&
    (filter.since === undefined || day >= filter.since) &&
    (filter.until === undefined || day <= filter.until)
  )
}

function promptHash(run: LedgerCriticRun): string {
  const hashes = run.prompt_hashes
  const lenses = Object.entries(run.lens_hashes).sort(([a], [b]) => a.localeCompare(b))
  return sha256(
    JSON.stringify([hashes.preamble, hashes.role, hashes.contract, hashes.schema, lenses]),
  ).slice(0, 12)
}

function criticKey(runId: string, criticId: string): string {
  return `${runId} ${criticId}`
}

function raiser(
  byCritic: ReadonlyMap<string, Tally>,
  finding: LedgerFinding,
  criticId: string,
): Tally {
  const tally = byCritic.get(criticKey(finding.run_id, criticId))
  if (tally === undefined) {
    throw new CritiqueError(
      'invalid-ledger',
      `a finding of run ${finding.run_id} names critic ${criticId}, which has no run line`,
    )
  }
  return tally
}

/**
 * What a removal rule counts a ledger line under: for `features`, the spec
 * it belongs to, and a line recorded before the ledger named the spec counts
 * under none; for `runs`, the run itself.
 */
const REMOVAL_UNITS: Readonly<
  Record<RemovalRule, (line: { readonly spec: string | undefined; readonly run_id: string }) => string | undefined>
> = {
  features: (line) => line.spec,
  runs: (line) => line.run_id,
}

/**
 * A stage is a candidate when its latest `features` features (distinct
 * specs, ordered by their latest run of the stage) brought no unique
 * accepted catch; a review stage, when its latest `features` runs did. A
 * stage that has run on fewer is not judged yet.
 */
function removalCandidates(
  runs: readonly LedgerCriticRun[],
  uniqueCatches: readonly LedgerFinding[],
  features: number,
): readonly RemovalCandidate[] {
  const candidates: RemovalCandidate[] = []
  for (const stage of CRITIQUE_STAGES) {
    const rule = STAGE_TABLE[stage].removal
    const unitOf = REMOVAL_UNITS[rule]
    const latest = new Map<string, string>()
    for (const run of runs) {
      const unit = unitOf(run)
      if (run.stage !== stage || unit === undefined) continue
      if ((latest.get(unit) ?? '') < run.recorded_at) latest.set(unit, run.recorded_at)
    }
    if (latest.size < features) continue
    const looked = [...latest]
      .sort(([, a], [, b]) => b.localeCompare(a))
      .slice(0, features)
      .map(([unit]) => unit)
    if (
      !uniqueCatches.some(
        (finding) => finding.stage === stage && looked.includes(unitOf(finding) ?? ''),
      )
    ) {
      candidates.push(rule === 'runs' ? { runs: looked, stage } : { features: looked, stage })
    }
  }
  return candidates
}

/**
 * For each routed client of a stage, the client off the roster whose
 * current prompt has the higher precision on that stage, when both current
 * cells have enough adjudicated findings. A client's current cell is its
 * latest prompt hash on the stage, since that is the prompt that runs now.
 */
function suggestions(
  cells: readonly StatsCell[],
  routed: (stage: CritiqueStage) => readonly string[],
): readonly string[] {
  const sentences: string[] = []
  for (const stage of CRITIQUE_STAGES) {
    const current = new Map<string, StatsCell>()
    for (const cell of cells) {
      if (cell.stage !== stage) continue
      const seen = current.get(cell.client)
      if (seen === undefined || seen.last_recorded_at < cell.last_recorded_at)
        current.set(cell.client, cell)
    }
    const eligible = [...current.values()].filter((cell) => !cell.below_threshold)
    const roster = routed(stage)
    for (const client of roster) {
      const incumbent = eligible.find((cell) => cell.client === client)
      if (incumbent === undefined) continue
      const [challenger] = eligible
        .filter(
          (cell) => !roster.includes(cell.client) && precisionOf(cell) > precisionOf(incumbent),
        )
        .sort((a, b) => precisionOf(b) - precisionOf(a))
      if (challenger === undefined) continue
      sentences.push(
        `${challenger.client} has higher precision than ${client} on the ${stage} stage over ` +
          `${String(challenger.adjudicated + incumbent.adjudicated)} findings ` +
          `(${precisionOf(challenger).toFixed(2)} against ${precisionOf(incumbent).toFixed(2)}); consider swapping.`,
      )
    }
  }
  return sentences
}

function precisionOf(cell: StatsCell): number {
  return cell.precision ?? 0
}

/** What the report says the figures cover. */
export interface StatsScope {
  readonly ledgerPath: string
  readonly filter: StatsFilter
  readonly thresholds: StatsThresholds
}

/** The report the skill shows the owner: one table per stage, the removal candidates, then the suggestions. */
export function renderStats(stats: CritiqueStats, scope: StatsScope): string {
  const { filter, thresholds } = scope
  const dates =
    filter.since === undefined && filter.until === undefined
      ? 'all'
      : `${filter.since ?? 'start'} to ${filter.until ?? 'today'}`
  const lines = [
    '# Critique stats',
    '',
    `Ledger: ${scope.ledgerPath}. Repository: ${filter.repository ?? 'all'}. Dates: ${dates}.`,
    `One row per critic client and prompt. A row with fewer than ${String(thresholds.min_adjudicated_findings)} adjudicated findings gets no suggestion.`,
  ]
  const wasted = wastedLine(stats.cells)
  if (wasted !== undefined) lines.push(wasted)
  if (stats.cells.length === 0) lines.push('', 'No critic runs in the ledger match.')
  for (const stage of CRITIQUE_STAGES) {
    const rows = stats.cells.filter((cell) => cell.stage === stage)
    if (rows.length === 0) continue
    lines.push(
      '',
      `## ${stage}`,
      '',
      '| client | prompt | runs | adjudicated | precision | unique | fabricated | cost per accepted | failed | timed out | wasted |',
      '|---|---|---|---|---|---|---|---|---|---|---|',
      ...rows
        .map((cell) =>
          [
            cell.client,
            cell.prompt_hash,
            String(cell.runs),
            `${String(cell.adjudicated)}${cell.below_threshold ? ' (too few)' : ''}`,
            cell.precision === null ? '-' : cell.precision.toFixed(2),
            String(cell.unique_catches),
            cell.fabrication_rate === null ? '-' : percent(cell.fabrication_rate),
            cell.cost_per_accepted_usd === null ? '-' : `$${cell.cost_per_accepted_usd.toFixed(4)}`,
            percent(cell.failure_rate),
            percent(cell.timeout_rate),
            cell.stopped_runs === 0 ? '-' : `$${cell.wasted_usd.toFixed(4)} (${String(cell.stopped_runs)})`,
          ].join(' | '),
        )
        .map((row) => `| ${row} |`),
    )
  }
  if (stats.cells.length > 0)
    lines.push(...calibrationLines(stats.cells, thresholds.min_adjudicated_findings))
  lines.push('', '## Removal candidates', '')
  lines.push(
    ...(stats.removal_candidates.length === 0
      ? ['none']
      : stats.removal_candidates.map(
          (candidate) => {
            const [looked, unit] = 'runs' in candidate ? [candidate.runs, 'runs'] : [candidate.features, 'features']
            return `- ${candidate.stage}: no unique accepted catch across its last ${String(looked.length)} ${unit} (${looked.join(', ')})`
          },
        )),
  )
  lines.push(
    '',
    '## Suggestions',
    '',
    ...(stats.suggestions.length === 0 ? ['none'] : stats.suggestions.map((s) => `- ${s}`)),
  )
  return `${lines.join('\n')}\n`
}

/** The calibration table: one row per cell with enough adjudicated findings, or the sentence saying none has. */
function calibrationLines(cells: readonly StatsCell[], minimum: number): readonly string[] {
  const rows = cells.flatMap((cell) => (cell.calibration === null ? [] : [{ calibration: cell.calibration, cell }]))
  if (rows.length === 0)
    return ['', '## Calibration', '', `No critic has ${String(minimum)} adjudicated findings yet.`]
  const share = (slice: PrecisionSlice): string =>
    slice.precision === null ? '-' : `${slice.precision.toFixed(2)} (${String(slice.adjudicated)})`
  return [
    '',
    '## Calibration',
    '',
    `Precision over time and by severity, for cells with ${String(minimum)} or more adjudicated findings. Earlier and recent are the older and newer half of the cell's findings; the count is in brackets.`,
    '',
    '| stage | client | prompt | adjudicated | earlier | recent | drift | critical | major | minor |',
    '|---|---|---|---|---|---|---|---|---|---|',
    ...rows.map(({ calibration, cell }) =>
      `| ${[
        cell.stage,
        cell.client,
        cell.prompt_hash,
        String(cell.adjudicated),
        share(calibration.earlier),
        share(calibration.recent),
        calibration.drift === null ? '-' : `${calibration.drift >= 0 ? '+' : ''}${calibration.drift.toFixed(2)}`,
        share(calibration.by_severity.critical),
        share(calibration.by_severity.major),
        share(calibration.by_severity.minor),
      ].join(' | ')} |`,
    ),
  ]
}

/** What every row's stopped runs cost together; undefined when no run was stopped. */
function wastedLine(cells: readonly StatsCell[]): string | undefined {
  const runs = cells.reduce((total, cell) => total + cell.stopped_runs, 0)
  if (runs === 0) return undefined
  const usd = cells.reduce((total, cell) => total + cell.wasted_usd, 0)
  const runsText = runs === 1 ? '1 run a price or time limit stopped before it' : `${String(runs)} runs a price or time limit stopped before they`
  return `Wasted: $${usd.toFixed(4)} on ${runsText} found anything.`
}

function percent(rate: number): string {
  return `${String(Math.round(rate * 100))}%`
}
