import type { FindingBody, PreVerdict } from './findings.js'

/**
 * A check id opens a lens bullet or a role rule: a backticked lower-case
 * slug at the start of a top-level bullet, followed by a colon, as in
 * "- `dependency-fails`: For each external dependency ...".
 */
const CHECK_BULLET = /^- `([a-z0-9]+(?:-[a-z0-9]+)*)`:/gm

const CHECK_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

/** Whether a check a finding names has the shape of a check id, rather than free text. */
export function isCheckId(check: string | undefined): check is string {
  return check !== undefined && CHECK_ID.test(check)
}

/** The check ids a critic's role and lens files declare, in the order they appear. */
export function declaredChecks(texts: readonly string[]): readonly string[] {
  return [...new Set(texts.flatMap((text) => [...text.matchAll(CHECK_BULLET)].map((match) => match[1] ?? '')))]
}

/**
 * Whether a finding names a check its critic's prompt declares. Returns a
 * hint when it names none, or one that no lens or role of that prompt
 * declares, and nothing when the check is one of `declared`.
 */
export function checkNamed(finding: Pick<FindingBody, 'check'>, declared: readonly string[]): PreVerdict | undefined {
  if (finding.check === undefined) return { reason: 'names no check', reason_code: 'no-check', verdict: 'hint' }
  if (declared.includes(finding.check)) return undefined
  return {
    reason: `names the check ${finding.check}, which no lens or role in its critic's prompt declares`,
    reason_code: 'unknown-check',
    verdict: 'hint',
  }
}
