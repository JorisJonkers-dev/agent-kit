import type { CriticRun } from './critic.js'
import type { DepthChoice } from './depth.js'
import type { ModelFamily } from './family.js'
import type { RunFinding } from './findings.js'
import type { DroppedCritic } from './routing.js'
import { SECOND_ROUND_AFTER_CRITICAL, type CritiqueStage, type Depth, type Round } from './stage.js'
import type { TicketCheckSummary } from './tickets.js'

export const VERDICTS = ['accepted', 'rejected', 'needs-human', 'hint'] as const
export type VerdictKind = (typeof VERDICTS)[number]

export interface Verdict {
  readonly finding_id: string
  readonly verdict: VerdictKind
  /** Free text for the report; never written to the ledger. */
  readonly reason?: string
  /** A short slug the ledger may keep, such as `fabricated-evidence`. */
  readonly reason_code?: string
  /** On a needs-human verdict: the owner has answered the question. */
  readonly answered?: boolean
}

export interface AdjudicatedFinding extends RunFinding {
  readonly verdict: Verdict
}

export interface AdjudicatedRun {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly round: Round
  readonly previous_run_id?: string
  readonly depth_choice: DepthChoice
  readonly repository: string
  /** The flow whose Vibe spend the run adds to. */
  readonly flow: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly critics: readonly CriticRun[]
  readonly dropped: readonly DroppedCritic[]
  readonly findings: readonly AdjudicatedFinding[]
  /** Only for the tickets stage: what the ticket checker found when the packet was frozen. */
  readonly ticket_check?: TicketCheckSummary
}

export type VerdictCounts = Readonly<Record<VerdictKind, number>>

export function countVerdicts(run: AdjudicatedRun): VerdictCounts {
  const counts = { accepted: 0, hint: 0, 'needs-human': 0, rejected: 0 }
  for (const finding of run.findings) counts[finding.verdict.verdict] += 1
  return counts
}

/**
 * More than three accepted critical findings call for a second round, unless
 * this was it. The decisions stage never calls for one: its accepted
 * findings are questions for the owner and change no artifact.
 */
export function secondRoundDue(run: AdjudicatedRun): boolean {
  const critical = run.findings.filter((f) => f.severity === 'critical' && f.verdict.verdict === 'accepted')
  return run.round === 1 && run.stage !== 'decisions' && critical.length > SECOND_ROUND_AFTER_CRITICAL
}

/**
 * Why a verdict does not fit its stage, if it does not. A question is
 * answered on a needs-human verdict. At the decisions stage every accepted
 * finding is a question for the owner, so the answer goes on the accepted
 * verdict, and needs-human is never given.
 */
export function stageVerdictError(stage: CritiqueStage, verdict: Verdict): string | undefined {
  const id = verdict.finding_id
  if (stage === 'decisions') {
    if (verdict.verdict === 'needs-human') return `${id}: the decisions stage asks every accepted gap as a question; give it accepted, not needs-human`
    if (verdict.answered !== undefined && verdict.verdict !== 'accepted') return `${id}: only an accepted verdict can be answered at the decisions stage`
    return undefined
  }
  return verdict.answered !== undefined && verdict.verdict !== 'needs-human' ? `${id}: only a needs-human verdict can be answered` : undefined
}

/** Critical questions the owner has not answered yet; while any is open, tickets must not be cut from the spec. */
export function openCriticalQuestions(run: AdjudicatedRun): readonly AdjudicatedFinding[] {
  return run.findings.filter((f) => f.severity === 'critical' && f.verdict.verdict === 'needs-human' && f.verdict.answered !== true)
}

/** The families of the critics that raised a finding. */
export function familiesOf(run: AdjudicatedRun, finding: RunFinding): readonly ModelFamily[] {
  const families = run.critics.filter((critic) => finding.raised_by.includes(critic.critic_id)).map((c) => c.family)
  return [...new Set(families)]
}

/**
 * The orchestrator's rejection of a finding with a strong signal: raised by
 * two or more families, or critical. A finding rejected for fabricated
 * evidence is not dissent: its evidence does not exist, and no orchestrator
 * ruled against it.
 */
export function isDissent(run: AdjudicatedRun, finding: AdjudicatedFinding): boolean {
  return (
    finding.verdict.verdict === 'rejected' &&
    finding.verdict.reason_code !== 'fabricated-evidence' &&
    (finding.severity === 'critical' || familiesOf(run, finding).length >= 2)
  )
}
