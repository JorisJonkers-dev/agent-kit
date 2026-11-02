import type { LoggedInvocation } from './budget.js'
import type { LedgerFinding, LedgerRunContext } from './ledger.js'
import { isReviewStage, type CritiqueStage, type ReviewStage } from './stage.js'
import { VERDICTS, type VerdictCounts } from './verdicts.js'

/** One review run as `/flow` lists it: a flow of its own, so its spend is its own. */
export interface ReviewRun {
  readonly stage: ReviewStage
  readonly run_id: string
  readonly round: number
  readonly recorded: boolean
  /** The recorded verdicts of its findings; all zero until the run is recorded. */
  readonly verdicts: VerdictCounts
  /** What its Vibe critics were charged. */
  readonly spent_usd: number
}

/** A run as its directory or the ledger shows it. */
export interface ReviewRunSource {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly round: number
  readonly recorded: boolean
}

/** Everything the list is computed from, already narrowed to one repository. */
export interface ReviewRunSources {
  /** The runs the run directories hold. */
  readonly directories: readonly ReviewRunSource[]
  /** The ledger's run lines, which still hold runs whose directory is gone. */
  readonly ledgerRuns: readonly Pick<LedgerRunContext, 'run_id' | 'stage' | 'round'>[]
  readonly findings: readonly Pick<LedgerFinding, 'run_id' | 'verdict'>[]
  readonly invocations: readonly Pick<LoggedInvocation, 'run_id' | 'kind' | 'charged_usd'>[]
}

/**
 * The review runs, newest first, at most `limit` of them. A run of a
 * critique stage belongs to a feature's flow and is left out. Nothing here
 * says a review is due: a review runs only when someone asks for one.
 */
export function reviewRuns(sources: ReviewRunSources, limit: number): readonly ReviewRun[] {
  const runs = new Map<string, ReviewRunSource>()
  for (const run of sources.directories) runs.set(run.run_id, run)
  for (const line of sources.ledgerRuns) runs.set(line.run_id, { ...line, ...runs.get(line.run_id), recorded: true })
  return [...runs.values()]
    .flatMap((run) => (isReviewStage(run.stage) ? [{ ...run, stage: run.stage }] : []))
    .sort((a, b) => b.run_id.localeCompare(a.run_id, 'en', { numeric: true }))
    .slice(0, limit)
    .map((run) => ({
      recorded: run.recorded,
      round: run.round,
      run_id: run.run_id,
      spent_usd: sources.invocations
        .filter((line) => line.kind === 'vibe' && line.run_id === run.run_id)
        .reduce((total, line) => total + line.charged_usd, 0),
      stage: run.stage,
      verdicts: verdictCounts(sources.findings.filter((finding) => finding.run_id === run.run_id)),
    }))
}

function verdictCounts(findings: readonly Pick<LedgerFinding, 'verdict'>[]): VerdictCounts {
  return Object.fromEntries(VERDICTS.map((verdict) => [verdict, findings.filter((finding) => finding.verdict === verdict).length])) as VerdictCounts
}
