import type { CriticRun } from './critic.js'
import type { ModelFamily } from './family.js'
import type { RunFinding } from './findings.js'
import type { DroppedCritic } from './routing.js'
import type { CritiqueStage, Depth } from './stage.js'

export const VERDICTS = ['accepted', 'rejected', 'needs-human', 'hint'] as const
export type VerdictKind = (typeof VERDICTS)[number]

export interface Verdict {
  readonly finding_id: string
  readonly verdict: VerdictKind
  /** Free text for the report; never written to the ledger. */
  readonly reason?: string
  /** A short slug the ledger may keep, such as `fabricated-evidence`. */
  readonly reason_code?: string
}

export interface AdjudicatedFinding extends RunFinding {
  readonly verdict: Verdict
}

export interface AdjudicatedRun {
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly repository: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly critics: readonly CriticRun[]
  readonly dropped: readonly DroppedCritic[]
  readonly findings: readonly AdjudicatedFinding[]
}

export type VerdictCounts = Readonly<Record<VerdictKind, number>>

export function countVerdicts(run: AdjudicatedRun): VerdictCounts {
  const counts = { accepted: 0, hint: 0, 'needs-human': 0, rejected: 0 }
  for (const finding of run.findings) counts[finding.verdict.verdict] += 1
  return counts
}

/** The families of the critics that raised a finding. */
export function familiesOf(run: AdjudicatedRun, finding: RunFinding): readonly ModelFamily[] {
  const families = run.critics.filter((critic) => finding.raised_by.includes(critic.critic_id)).map((c) => c.family)
  return [...new Set(families)]
}

/** A rejection with a strong signal: raised by two or more families, or critical. */
export function isDissent(run: AdjudicatedRun, finding: AdjudicatedFinding): boolean {
  return (
    finding.verdict.verdict === 'rejected' && (finding.severity === 'critical' || familiesOf(run, finding).length >= 2)
  )
}
