export const SEVERITIES = ['critical', 'major', 'minor'] as const
export type Severity = (typeof SEVERITIES)[number]

export const CATEGORIES = [
  'unasked-question',
  'contradiction',
  'omission',
  'invention',
  'untestable',
  'failure-mode',
  'security',
  'structure',
  'bug',
  'integration',
] as const
export type Category = (typeof CATEGORIES)[number]

export const EVIDENCE_KINDS = ['quote', 'input', 'failing-test', 'trace'] as const
export type EvidenceKind = (typeof EVIDENCE_KINDS)[number]

export interface Evidence {
  readonly kind: EvidenceKind
  readonly value: string
  readonly file?: string
}

/** One finding as a critic reported it, after validation against the output contract. */
export interface CriticFinding {
  readonly id: string
  readonly severity: Severity
  readonly category: Category
  readonly location: string
  readonly claim: string
  readonly evidence: readonly Evidence[]
  readonly suggestion?: string
}

export type FindingsFailure = 'no-findings-block' | 'unparseable-findings' | 'invalid-findings' | 'missing-no-findings-reason'

export type FindingsParse =
  | { readonly ok: true; readonly findings: readonly CriticFinding[]; readonly no_findings_reason?: string }
  | { readonly ok: false; readonly failure: FindingsFailure; readonly detail: string }

const FENCED_JSON = /```json[ \t]*\r?\n([\s\S]*?)\r?\n?```/g

/** Reads the last ```json block of a critic's final message and checks it against the contract. */
export function parseFindings(text: string): FindingsParse {
  const blocks = [...text.matchAll(FENCED_JSON)]
  const last = blocks.at(-1)
  if (last === undefined) return { detail: 'the reply has no ```json block', failure: 'no-findings-block', ok: false }
  let value: unknown
  try {
    value = JSON.parse(last[1] ?? '')
  } catch (error) {
    return { detail: String(error), failure: 'unparseable-findings', ok: false }
  }
  const findings = isRecord(value) ? value.findings : undefined
  if (!Array.isArray(findings)) {
    return { detail: 'the block has no findings list', failure: 'invalid-findings', ok: false }
  }
  const invalid = findings.findIndex((item) => !isFinding(item))
  if (invalid !== -1) {
    return { detail: `finding ${String(invalid + 1)} does not match the contract`, failure: 'invalid-findings', ok: false }
  }
  const reason = isRecord(value) && typeof value.no_findings_reason === 'string' ? value.no_findings_reason.trim() : ''
  if (findings.length === 0 && reason.length === 0) {
    return { detail: 'an empty findings list needs no_findings_reason', failure: 'missing-no-findings-reason', ok: false }
  }
  return {
    findings: findings as readonly CriticFinding[],
    ok: true,
    ...(reason.length > 0 ? { no_findings_reason: reason } : {}),
  }
}

function isFinding(value: unknown): value is CriticFinding {
  return (
    isRecord(value) &&
    isText(value.id) &&
    oneOf(SEVERITIES, value.severity) &&
    oneOf(CATEGORIES, value.category) &&
    isText(value.location) &&
    isText(value.claim) &&
    Array.isArray(value.evidence) &&
    value.evidence.every(isEvidence) &&
    (value.suggestion === undefined || typeof value.suggestion === 'string')
  )
}

function isEvidence(value: unknown): value is Evidence {
  return (
    isRecord(value) &&
    oneOf(EVIDENCE_KINDS, value.kind) &&
    isText(value.value) &&
    (value.file === undefined || isText(value.file))
  )
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isText(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

function oneOf<T extends string>(allowed: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value)
}
