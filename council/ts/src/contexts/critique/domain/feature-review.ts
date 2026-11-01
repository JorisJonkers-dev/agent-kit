import { SEVERITIES, type Severity } from './findings.js'
import { JsonFields } from './json.js'

/** One accepted feature finding and the follow-up ticket it became. */
export interface FollowUp {
  readonly finding_id: string
  readonly severity: Severity
  /** The ticket as its tracker numbers it: `#301`, or the `NN` of a local ticket file. */
  readonly ticket: string
  /** Where it lives: `host/owner/repo#<number>` for an issue, its repository path for a file. */
  readonly reference: string
  /** Whether the ticket was open when the run was recorded. */
  readonly open: boolean
}

/** A critical follow-up, from this run or an earlier one of the same spec, still open: what holds the gate shut. */
export interface OpenCriticalFollowUp {
  readonly finding_id: string
  /** The run that accepted the finding. */
  readonly run_id: string
  readonly ticket: string
  readonly reference: string
}

/**
 * A feature review's gate, recorded for the head it reviewed so a script can
 * read it without any model call: `gate.json` in the run, and one line of
 * the critique directory's gate index.
 */
export interface GateRecord {
  readonly version: 1
  readonly stage: 'feature-review'
  readonly run_id: string
  readonly recorded_at: string
  readonly repository: string
  /** The spec's reference, as the manifest's artifact names it. */
  readonly spec: string
  readonly base_commit: string
  /** The commit the gate holds for; a new head needs a new run. */
  readonly head_commit: string
  readonly gate: 'pass' | 'fail'
  readonly critical_open: number
  /** The report's first line: `GATE: PASS` or `GATE: FAIL (<n> critical)`. */
  readonly line: string
  /** Every accepted finding of this run and its follow-up ticket. */
  readonly follow_ups: readonly FollowUp[]
  readonly open_critical: readonly OpenCriticalFollowUp[]
}

export function gateLine(criticalOpen: number): string {
  return criticalOpen === 0 ? 'GATE: PASS' : `GATE: FAIL (${String(criticalOpen)} critical)`
}

/**
 * The critical follow-ups that hold the gate: this run's, then every
 * earlier run's for the same spec, each counted once by ticket and only
 * while its ticket is open.
 */
export function openCriticalFollowUps(
  runId: string,
  current: readonly FollowUp[],
  earlier: readonly GateRecord[],
  isOpen: (reference: string) => boolean,
): readonly OpenCriticalFollowUp[] {
  const critical = [
    ...current.map((followUp) => ({ followUp, from: runId })),
    ...earlier.flatMap((record) => record.follow_ups.map((followUp) => ({ followUp, from: record.run_id }))),
  ].filter(({ followUp }) => followUp.severity === 'critical')
  const seen = new Set<string>()
  const open: OpenCriticalFollowUp[] = []
  for (const { followUp, from } of critical) {
    if (seen.has(followUp.reference)) continue
    seen.add(followUp.reference)
    if (isOpen(followUp.reference)) {
      open.push({ finding_id: followUp.finding_id, reference: followUp.reference, run_id: from, ticket: followUp.ticket })
    }
  }
  return open
}

/** Reads one gate record, as `gate.json` or a line of the gate index holds it. */
export function parseGateRecord(text: string): GateRecord {
  const file = JsonFields.parse(text)
  return {
    base_commit: file.string('base_commit'),
    critical_open: file.number('critical_open'),
    follow_ups: file.list('follow_ups', (followUp) => ({
      finding_id: followUp.string('finding_id'),
      open: followUp.boolean('open'),
      reference: followUp.string('reference'),
      severity: followUp.oneOf('severity', SEVERITIES),
      ticket: followUp.string('ticket'),
    })),
    gate: file.oneOf('gate', ['pass', 'fail'] as const),
    head_commit: file.string('head_commit'),
    line: file.string('line'),
    open_critical: file.list('open_critical', (followUp) => ({
      finding_id: followUp.string('finding_id'),
      reference: followUp.string('reference'),
      run_id: followUp.string('run_id'),
      ticket: followUp.string('ticket'),
    })),
    recorded_at: file.string('recorded_at'),
    repository: file.string('repository'),
    run_id: file.string('run_id'),
    spec: file.string('spec'),
    stage: file.literal('stage', 'feature-review'),
    version: file.literal('version', 1),
  }
}

/** Ticket statuses that mean a local markdown ticket is done. */
const CLOSED_STATUSES: readonly string[] = ['closed', 'done', 'resolved', 'wontfix']

/**
 * Whether a local markdown ticket is still open: its first `Status:` line
 * (`**Status:** ready-for-agent`, as `/to-tickets` writes it) names none of
 * the closed statuses. A ticket with no status line is open.
 */
export function localTicketOpen(text: string): boolean {
  const status = /^[ \t>]*\**Status\**[ \t]*:\**[ \t]*(.*)$/im.exec(text)?.[1]
  const value = status?.replace(/[*_`]/g, '').trim().toLowerCase()
  return value === undefined || !CLOSED_STATUSES.includes(value)
}
