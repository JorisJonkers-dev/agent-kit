import type { CriticRun } from './critic.js'
import type { DepthChoice } from './depth.js'
import type { GateRecord } from './feature-review.js'
import type { ModelFamily } from './family.js'
import type { RunFinding } from './findings.js'
import type { PacketFile } from './manifest.js'
import type { DroppedCritic } from './routing.js'
import { SECOND_ROUND_AFTER_CRITICAL, type CritiqueStage, type Depth, type Round } from './stage.js'

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
  /** On an accepted feature-review verdict: the follow-up ticket the finding became, as its tracker names it. */
  readonly follow_up?: string
  /** On a feature-review verdict accepted by a cross-ticket trace: the packet's ticket files the trace crossed. */
  readonly trace_tickets?: readonly string[]
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
  /** A feature review's gate, for the head it reviewed. */
  readonly gate?: GateRecord
}

export type VerdictCounts = Readonly<Record<VerdictKind, number>>

export function countVerdicts(run: AdjudicatedRun): VerdictCounts {
  const counts = { accepted: 0, hint: 0, 'needs-human': 0, rejected: 0 }
  for (const finding of run.findings) counts[finding.verdict.verdict] += 1
  return counts
}

/** More than three accepted critical findings call for a second round, unless this was it. */
export function secondRoundDue(run: AdjudicatedRun): boolean {
  const critical = run.findings.filter((f) => f.severity === 'critical' && f.verdict.verdict === 'accepted')
  return run.round === 1 && critical.length > SECOND_ROUND_AFTER_CRITICAL
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

/**
 * The two ways a feature finding can be accepted: a failing test the
 * orchestrator ran, or a trace it walked across code from two or more
 * tickets. Anything less stays a claim.
 */
export const FEATURE_ACCEPT_REASONS = ['test-fails', 'cross-ticket-trace'] as const

/**
 * What is wrong with a verdict given at this stage, if anything. Only a
 * feature review takes `follow_up` and `trace_tickets`; there an accepted
 * finding must name the follow-up ticket it became and the evidence that
 * accepted it, and a cross-ticket trace the packet tickets it crossed.
 */
export function verdictProblem(stage: CritiqueStage, verdict: Verdict, packetFiles: readonly PacketFile[]): string | undefined {
  const id = verdict.finding_id
  const extras = verdict.follow_up !== undefined || verdict.trace_tickets !== undefined
  if (stage !== 'feature-review') return extras ? `${id}: follow_up and trace_tickets belong to a feature-review verdict` : undefined
  if (verdict.verdict !== 'accepted') return extras ? `${id}: only an accepted feature finding has a follow_up or trace_tickets` : undefined
  const reason = verdict.reason_code
  if (reason !== 'test-fails' && reason !== 'cross-ticket-trace') {
    return `${id}: a feature finding is accepted only with reason_code ${FEATURE_ACCEPT_REASONS.join(' or ')}`
  }
  if (reason === 'cross-ticket-trace') {
    const tickets = new Set(packetFiles.filter((file) => file.kind === 'ticket').map((file) => file.path))
    const crossed = new Set(verdict.trace_tickets ?? [])
    if (crossed.size < 2 || [...crossed].some((path) => !tickets.has(path))) {
      return `${id}: a cross-ticket-trace names at least two tickets of the packet in trace_tickets`
    }
  } else if (verdict.trace_tickets !== undefined) {
    return `${id}: trace_tickets belong to a cross-ticket-trace`
  }
  return verdict.follow_up === undefined ? `${id}: an accepted feature finding names its follow-up ticket in follow_up` : undefined
}
