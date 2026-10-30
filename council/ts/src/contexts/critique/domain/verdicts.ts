import type { ModelFamily } from './family-allowlist.js'
import type { Category, Evidence, Severity } from './findings.js'
import type { PromptHashes } from './prompt.js'
import type { DroppedCritic } from './routing.js'

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

/** What record needs to know about each critic run. */
export interface RecordedCritic {
  readonly critic_id: string
  readonly client: string
  readonly family: ModelFamily
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
  readonly status: 'ok' | 'failed'
  readonly failure?: string
  readonly detail?: string
  readonly no_findings_reason?: string
  readonly exit_code: number
  readonly cost_usd: number | null
  readonly duration_ms: number
  readonly finding_count: number
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
}

export interface RecordedFinding {
  readonly id: string
  readonly raised_by: readonly string[]
  readonly severity: Severity
  readonly category: Category
  readonly location: string
  readonly claim: string
  readonly evidence: readonly Evidence[]
  readonly suggestion?: string
}

export interface AdjudicatedRun {
  readonly run_id: string
  readonly stage: string
  readonly depth: string
  readonly repository: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly critics: readonly RecordedCritic[]
  readonly dropped: readonly DroppedCritic[]
  readonly findings: readonly (RecordedFinding & { readonly verdict: Verdict })[]
}

export type VerdictCounts = Readonly<Record<VerdictKind, number>>

export function countVerdicts(run: AdjudicatedRun): VerdictCounts {
  const counts = { accepted: 0, hint: 0, 'needs-human': 0, rejected: 0 }
  for (const finding of run.findings) counts[finding.verdict.verdict] += 1
  return counts
}

/** The families of the critics that raised a finding. */
export function familiesOf(run: AdjudicatedRun, finding: RecordedFinding): readonly ModelFamily[] {
  const families = run.critics.filter((critic) => finding.raised_by.includes(critic.critic_id)).map((c) => c.family)
  return [...new Set(families)]
}

/** A rejection with a strong signal: raised by two or more families, or critical. */
export function isDissent(run: AdjudicatedRun, finding: AdjudicatedRun['findings'][number]): boolean {
  return (
    finding.verdict.verdict === 'rejected' && (finding.severity === 'critical' || familiesOf(run, finding).length >= 2)
  )
}
