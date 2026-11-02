import type { CriticRun } from './critic.js'
import type { DepthChoice } from './depth.js'
import type { GateRecord } from './feature-review.js'
import type { ModelFamily } from './family.js'
import type { RunFinding, Severity } from './findings.js'
import type { PacketFile } from './manifest.js'
import type { RefuteRecord } from './refute.js'
import type { DroppedCritic } from './routing.js'
import { SECOND_ROUND_AFTER_CRITICAL, type CritiqueStage, type Depth, type Round } from './stage.js'
import { STAGE_TABLE } from './stage-table.js'
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
  /** On an accepted feature-review verdict: the follow-up ticket the finding became, as its tracker names it. */
  readonly follow_up?: string
  /** On a feature-review verdict accepted by a cross-ticket trace: the packet's ticket files the trace crossed. */
  readonly trace_tickets?: readonly string[]
  /** On a ticket-review verdict accepted by `test-fails`: the repository path the test was kept at, in the ticket's change. */
  readonly regression_test?: string
  /** On an accepted ticket-review verdict: the finding is fixed in the change about to be committed. */
  readonly fixed?: boolean
  /** The orchestrator's override of the severity the finding's check fixes; it needs `severity_reason`. */
  readonly severity?: Severity
  /** Why the fixed severity was overridden: a short slug, so the ledger can keep it beside the override. */
  readonly severity_reason?: string
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
  /**
   * The spec the run belongs to, as the artifact's reference names it
   * (`host/owner/repo#<n>` or a repository path). Stats counts features by it.
   */
  readonly spec: string
  /** The flow whose Vibe spend the run adds to. */
  readonly flow: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly critics: readonly CriticRun[]
  readonly dropped: readonly DroppedCritic[]
  readonly findings: readonly AdjudicatedFinding[]
  /** Only for the tickets stage: what the ticket checker found when the packet was frozen. */
  readonly ticket_check?: TicketCheckSummary
  /** A feature review's gate, for the head it reviewed. */
  readonly gate?: GateRecord
  /** The refute round's results, when the run had one; already applied to `findings`. */
  readonly refutes?: readonly RefuteRecord[]
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
  return run.round === 1 && STAGE_TABLE[run.stage].secondRound && critical.length > SECOND_ROUND_AFTER_CRITICAL
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

/**
 * Why a verdict's severity override does not fit its finding, if it does
 * not. Only a severity the finding's check fixes can be overridden, to a
 * different level and with a reason; the critic's own rating is not the
 * orchestrator's to change.
 */
export function severityOverrideProblem(finding: RunFinding, verdict: Verdict): string | undefined {
  const id = verdict.finding_id
  if (verdict.severity === undefined) {
    return verdict.severity_reason === undefined ? undefined : `${id}: a severity_reason belongs to a severity override`
  }
  if (finding.declared_severity === undefined) return `${id}: only a finding whose check fixes its severity can be overridden`
  if (verdict.severity === finding.declared_severity) return `${id}: the override repeats the declared severity ${verdict.severity}`
  return verdict.severity_reason === undefined ? `${id}: a severity override needs a severity_reason` : undefined
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
  return featureVerdictProblem(stage, verdict, packetFiles) ?? ticketVerdictProblem(stage, verdict)
}

function featureVerdictProblem(stage: CritiqueStage, verdict: Verdict, packetFiles: readonly PacketFile[]): string | undefined {
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

/**
 * The two ways a ticket finding can be accepted: a failing test the
 * orchestrator ran in a scratch worktree, or a trace it walked line by line.
 */
export const TICKET_ACCEPT_REASONS = ['test-fails', 'trace-walked'] as const

/**
 * What is wrong with a verdict's ticket-review fields, if anything. Only a
 * ticket review takes `regression_test` and `fixed`, and only on an accepted
 * finding; there a finding is accepted by a failing test or a walked trace,
 * and one a test decided names where the test was kept.
 */
function ticketVerdictProblem(stage: CritiqueStage, verdict: Verdict): string | undefined {
  const id = verdict.finding_id
  const extras = verdict.regression_test !== undefined || verdict.fixed !== undefined
  if (stage !== 'ticket-review') return extras ? `${id}: regression_test and fixed belong to a ticket-review verdict` : undefined
  if (verdict.verdict !== 'accepted') return extras ? `${id}: only an accepted ticket finding has a regression_test or is fixed` : undefined
  const reason = verdict.reason_code
  if (reason !== 'test-fails' && reason !== 'trace-walked') {
    return `${id}: a ticket finding is accepted only with reason_code ${TICKET_ACCEPT_REASONS.join(' or ')}`
  }
  if (reason === 'test-fails' && verdict.regression_test === undefined) {
    return `${id}: a finding accepted by test-fails names the test kept in the ticket's change in regression_test`
  }
  if (reason !== 'test-fails' && verdict.regression_test !== undefined) return `${id}: regression_test belongs to a finding accepted by test-fails`
  return undefined
}

/** #181 US53: accepted critical findings of a ticket review not yet fixed; while any is, the change must not be committed. */
export function unfixedCritical(run: Pick<AdjudicatedRun, 'stage' | 'findings'>): readonly AdjudicatedFinding[] {
  if (run.stage !== 'ticket-review') return []
  return run.findings.filter((f) => f.severity === 'critical' && f.verdict.verdict === 'accepted' && f.verdict.fixed !== true)
}
