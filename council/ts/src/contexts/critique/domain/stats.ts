import type { Ledger, LedgerCriticRun, LedgerFinding, LedgerRunContext } from './ledger.js'
import { sha256 } from './manifest.js'
import { CritiqueError } from './outcomes.js'
import type { StatsThresholds } from './routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from './stage.js'

/** Which ledger lines the figures cover. Dates are `YYYY-MM-DD`, both ends included, read from `recorded_at` in UTC. */
export interface StatsFilter {
  /** The allowlist key, `host/owner/repo`; compared ignoring case. */
  readonly repository?: string
  readonly since?: string
  readonly until?: string
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
}

/** A stage whose latest features brought no accepted finding that only one critic client raised. */
export interface RemovalCandidate {
  readonly stage: CritiqueStage
  /** The features (spec references) looked at, latest first. */
  readonly features: readonly string[]
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
  }

  addFinding(finding: LedgerFinding, unique: boolean): void {
    this.findings += 1
    if (finding.verdict === 'accepted') this.accepted += 1
    if (finding.verdict === 'rejected') this.rejected += 1
    if (finding.verdict === 'hint') this.hints += 1
    if (finding.verdict === 'needs-human') this.needsHuman += 1
    if (finding.verdict === 'accepted' && unique) this.unique += 1
    if (finding.pre_verdict === 'fabricated-evidence') this.fabricated += 1
  }

  cell(thresholds: StatsThresholds): StatsCell {
    const adjudicated = this.accepted + this.rejected
    const priced = this.runs - this.unpriced
    return {
      accepted: this.accepted,
      adjudicated,
      below_threshold: adjudicated < thresholds.min_adjudicated_findings,
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
      timed_out_runs: this.timedOut,
      timeout_rate: this.timedOut / this.runs,
      unique_catches: this.unique,
      unpriced_runs: this.unpriced,
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
 * A stage is a candidate when its latest `features` features (distinct
 * specs, ordered by their latest run of the stage) brought no unique
 * accepted catch. A stage that has run on fewer features is not judged yet.
 */
function removalCandidates(
  runs: readonly LedgerCriticRun[],
  uniqueCatches: readonly LedgerFinding[],
  features: number,
): readonly RemovalCandidate[] {
  const candidates: RemovalCandidate[] = []
  for (const stage of CRITIQUE_STAGES) {
    const latest = new Map<string, string>()
    for (const run of runs) {
      if (run.stage !== stage || run.spec === undefined) continue
      if ((latest.get(run.spec) ?? '') < run.recorded_at) latest.set(run.spec, run.recorded_at)
    }
    if (latest.size < features) continue
    const looked = [...latest]
      .sort(([, a], [, b]) => b.localeCompare(a))
      .slice(0, features)
      .map(([spec]) => spec)
    if (
      !uniqueCatches.some(
        (finding) => finding.stage === stage && looked.includes(finding.spec ?? ''),
      )
    ) {
      candidates.push({ features: looked, stage })
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
  if (stats.cells.length === 0) lines.push('', 'No critic runs in the ledger match.')
  for (const stage of CRITIQUE_STAGES) {
    const rows = stats.cells.filter((cell) => cell.stage === stage)
    if (rows.length === 0) continue
    lines.push(
      '',
      `## ${stage}`,
      '',
      '| client | prompt | runs | adjudicated | precision | unique | fabricated | cost per accepted | failed | timed out |',
      '|---|---|---|---|---|---|---|---|---|---|',
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
          ].join(' | '),
        )
        .map((row) => `| ${row} |`),
    )
  }
  lines.push('', '## Removal candidates', '')
  lines.push(
    ...(stats.removal_candidates.length === 0
      ? ['none']
      : stats.removal_candidates.map(
          (candidate) =>
            `- ${candidate.stage}: no unique accepted catch across its last ${String(candidate.features.length)} features (${candidate.features.join(', ')})`,
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

function percent(rate: number): string {
  return `${String(Math.round(rate * 100))}%`
}
