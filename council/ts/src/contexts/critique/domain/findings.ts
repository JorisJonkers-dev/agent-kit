import { isJsonObject, JsonFields } from './json.js'

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

/** What a finding claims, and the evidence for it. */
export interface FindingBody {
  readonly severity: Severity
  readonly category: Category
  readonly location: string
  readonly claim: string
  readonly evidence: readonly Evidence[]
  readonly suggestion?: string
}

/** One finding as a critic reported it, after validation against the output contract. */
export interface CriticFinding extends FindingBody {
  readonly id: string
}

/** A finding as the orchestrator receives it: the critic's fields plus who raised it. */
export interface RunFinding extends FindingBody {
  readonly id: string
  readonly critic_finding_id: string
  readonly raised_by: readonly string[]
}

export const FINDINGS_FAILURES = [
  'no-findings-block',
  'unparseable-findings',
  'invalid-findings',
  'missing-no-findings-reason',
] as const
export type FindingsFailure = (typeof FINDINGS_FAILURES)[number]

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
  let findings: readonly CriticFinding[]
  try {
    findings = JsonFields.of(value, 'the block').list('findings', readCriticFinding)
  } catch (error) {
    return { detail: String(error), failure: 'invalid-findings', ok: false }
  }
  const reason = isJsonObject(value) && typeof value.no_findings_reason === 'string' ? value.no_findings_reason.trim() : ''
  if (findings.length === 0 && reason.length === 0) {
    return { detail: 'an empty findings list needs no_findings_reason', failure: 'missing-no-findings-reason', ok: false }
  }
  return {
    findings,
    ok: true,
    ...(reason.length > 0 ? { no_findings_reason: reason } : {}),
  }
}

/** The fields of a finding, read against the output contract. */
export function readFindingBody(fields: JsonFields): FindingBody {
  const suggestion = fields.optionalText('suggestion')
  return {
    category: fields.oneOf('category', CATEGORIES),
    claim: fields.string('claim'),
    evidence: fields.list('evidence', readEvidence),
    location: fields.string('location'),
    severity: fields.oneOf('severity', SEVERITIES),
    ...(suggestion === undefined ? {} : { suggestion }),
  }
}

export function readRunFinding(fields: JsonFields): RunFinding {
  return {
    ...readFindingBody(fields),
    critic_finding_id: fields.string('critic_finding_id'),
    id: fields.string('id'),
    raised_by: fields.strings('raised_by'),
  }
}

function readCriticFinding(fields: JsonFields): CriticFinding {
  return { ...readFindingBody(fields), id: fields.string('id') }
}

function readEvidence(fields: JsonFields): Evidence {
  const file = fields.optionalString('file')
  return {
    kind: fields.oneOf('kind', EVIDENCE_KINDS),
    value: fields.string('value'),
    ...(file === undefined ? {} : { file }),
  }
}
