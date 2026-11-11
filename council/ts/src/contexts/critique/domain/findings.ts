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
  'drift',
  'stale',
  'undocumented-decision',
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
  /**
   * The check id of the lens bullet or role rule the finding comes from. The
   * output contract requires it; a finding without one is still read, and
   * fan-out gives it a hint.
   */
  readonly check?: string
}

/** One finding as a critic reported it, after validation against the output contract. */
export interface CriticFinding extends FindingBody {
  readonly id: string
}

/**
 * The verdict fan-out's evidence check gives a finding before the
 * orchestrator sees it. A quote that is not in the packet is rejected as
 * fabricated evidence, for good. A finding with nothing to check is a hint,
 * which the orchestrator may still accept or reject after checking it.
 */
export type PreVerdict =
  | { readonly verdict: 'rejected'; readonly reason_code: MechanicalRejection; readonly reason: string }
  | { readonly verdict: 'hint'; readonly reason_code: HintCode | CheckHintCode; readonly reason: string }

/**
 * The codes of a mechanical rejection, which the orchestrator cannot overturn.
 * `fabricated-evidence`: a quote or a cited line the packet does not hold. The
 * rest are a recovered decision's (`decision-recovery`): `no-code-trace` cites
 * no file of `tree/`, `single-site` cites only one, and `invented-rationale`
 * gives a reason with neither a quote of `history/` nor the plain statement
 * that no rationale was found.
 */
export const RECOVERY_REJECTIONS = ['no-code-trace', 'single-site', 'invented-rationale'] as const
export const MECHANICAL_REJECTIONS = ['fabricated-evidence', ...RECOVERY_REJECTIONS] as const
export type MechanicalRejection = (typeof MECHANICAL_REJECTIONS)[number]

export const HINT_CODES = [
  'no-evidence',
  'no-spec-quote',
  'no-decision-quote',
  'no-decision-or-scenario',
  'no-test-or-trace',
  'no-ticket-rule',
  'no-ticket-id',
  'no-scenario',
  'not-a-decision-gap',
  'no-record-quote',
  'one-file-contradiction',
  'needs-tree',
  'not-a-candidate',
  'no-layer-config',
  'no-trace',
  'verdict-word',
  'no-rule-or-consequence',
  'no-observable',
] as const
export type HintCode = (typeof HINT_CODES)[number]

/** Hints about the check a finding names, given after the stage's evidence rule found nothing to say. */
export const CHECK_HINT_CODES = ['no-check', 'unknown-check'] as const
export type CheckHintCode = (typeof CHECK_HINT_CODES)[number]

/** A finding as the orchestrator receives it: the critic's fields plus who raised it. */
export interface RunFinding extends FindingBody {
  readonly id: string
  readonly critic_finding_id: string
  readonly raised_by: readonly string[]
  readonly pre_verdict?: PreVerdict
  /**
   * The severity the finding's check fixes, set by fan-out in place of the
   * critic's. Only the orchestrator's verdict can change it, with a reason.
   */
  readonly declared_severity?: Severity
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
  return readFindings(value)
}

/** Checks a findings object, already parsed from JSON, against the output contract. */
export function readFindings(value: unknown): FindingsParse {
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
  // An empty check is no check: it earns a hint, not a failed review.
  const check = fields.optionalText('check')?.trim() ?? ''
  return {
    category: fields.oneOf('category', CATEGORIES),
    ...(check === '' ? {} : { check }),
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
    ...(fields.has('declared_severity') ? { declared_severity: fields.oneOf('declared_severity', SEVERITIES) } : {}),
    ...(fields.has('pre_verdict') ? { pre_verdict: readPreVerdict(fields.object('pre_verdict')) } : {}),
  }
}

function readPreVerdict(fields: JsonFields): PreVerdict {
  const reason = fields.string('reason')
  if (fields.oneOf('verdict', ['rejected', 'hint']) === 'rejected') {
    return { reason, reason_code: fields.oneOf('reason_code', MECHANICAL_REJECTIONS), verdict: 'rejected' }
  }
  return { reason, reason_code: fields.oneOf('reason_code', [...HINT_CODES, ...CHECK_HINT_CODES]), verdict: 'hint' }
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
