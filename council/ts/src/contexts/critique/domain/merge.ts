import { SEVERITIES, type CriticFinding, type Evidence, type RunFinding } from './findings.js'

/** The findings one critic of a run reported. */
export interface CriticFindings {
  readonly critic_id: string
  readonly findings: readonly CriticFinding[]
}

/**
 * Merges findings that point at the same location and claim the same defect,
 * across critics and within one, so the orchestrator reads each problem once.
 * A merged finding keeps every critic that raised it, so agreement between
 * families stays visible, and takes the highest severity any of them gave.
 *
 * Two findings are the same defect when their locations match, ignoring case,
 * spacing and quotes, their categories match, and they share a quote (one
 * quote containing the other counts, since critics cut quotes differently).
 * Findings without quotes are the same defect only when their claims match.
 * Merged findings are numbered `F1`, `F2`, ... in the order first raised.
 */
export function mergeFindings(perCritic: readonly CriticFindings[]): readonly RunFinding[] {
  const merged: MutableFinding[] = []
  for (const { critic_id, findings } of perCritic) {
    for (const finding of findings) {
      const same = merged.find((candidate) => sameDefect(candidate, finding))
      if (same === undefined) {
        const { id, ...fields } = finding
        merged.push({ ...fields, critic_finding_id: id, raised_by: [critic_id] })
        continue
      }
      if (!same.raised_by.includes(critic_id)) same.raised_by = [...same.raised_by, critic_id]
      same.evidence = [...same.evidence, ...finding.evidence.filter((e) => !same.evidence.some((known) => sameEvidence(known, e)))]
      if (rank(finding.severity) < rank(same.severity)) same.severity = finding.severity
      if (same.suggestion === undefined && finding.suggestion !== undefined) same.suggestion = finding.suggestion
    }
  }
  return merged.map((finding, index) => ({ ...finding, id: `F${String(index + 1)}` }))
}

type MutableFinding = { -readonly [Key in keyof Omit<RunFinding, 'id'>]: Omit<RunFinding, 'id'>[Key] }

function sameDefect(a: Omit<RunFinding, 'id'>, b: CriticFinding): boolean {
  if (a.category !== b.category || normal(a.location) !== normal(b.location)) return false
  const quotesA = quotes(a.evidence)
  const quotesB = quotes(b.evidence)
  if (quotesA.length === 0 && quotesB.length === 0) return normal(a.claim) === normal(b.claim)
  return quotesA.some((qa) => quotesB.some((qb) => qa.includes(qb) || qb.includes(qa)))
}

function quotes(evidence: readonly Evidence[]): readonly string[] {
  return evidence.filter((e) => e.kind === 'quote').map((e) => normal(e.value))
}

function sameEvidence(a: Evidence, b: Evidence): boolean {
  return a.kind === b.kind && a.value === b.value && a.file === b.file
}

/** Lower case, single spaces, no surrounding quotes or trailing punctuation. */
function normal(text: string): string {
  return text
    .toLowerCase()
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^["'`“‘]+|["'`”’.,;:]+$/g, '')
}

function rank(severity: RunFinding['severity']): number {
  return SEVERITIES.indexOf(severity)
}
