import { SEVERITIES, type FindingBody, type PreVerdict, type RunFinding, type Severity } from './findings.js'

/**
 * A check id opens a lens bullet or a role rule: a backticked lower-case
 * slug at the start of a top-level bullet, followed by a colon, as in
 * "- `dependency-fails`: For each external dependency ...". A check may
 * declare a fixed severity in brackets before the colon, as in
 * "- `no-rollback` (critical): ...". A check that reads code opens its text
 * with "Needs `tree/`.", as in "- `no-confirmation` (minor): Needs `tree/`. ...".
 */
const CHECK_BULLET = /^- `([a-z0-9]+(?:-[a-z0-9]+)*)`(?: \((critical|major|minor)\))?:( Needs `tree\/`\.)?/gm

/** A check a critic's role or lens declares, the severity it fixes, if it fixes one, and whether it needs code under `tree/`. */
export interface DeclaredCheck {
  readonly id: string
  readonly severity?: Severity
  readonly needsTree?: true
}

const CHECK_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

/** Whether a check a finding names has the shape of a check id, rather than free text. */
export function isCheckId(check: string | undefined): check is string {
  return check !== undefined && CHECK_ID.test(check)
}

/** The checks a critic's role and lens files declare, in the order they appear; an id declared twice counts once, as first declared. */
export function declaredChecks(texts: readonly string[]): readonly DeclaredCheck[] {
  const checks = new Map<string, DeclaredCheck>()
  for (const [, id = '', named, tree] of texts.flatMap((text) => [...text.matchAll(CHECK_BULLET)])) {
    const severity = SEVERITIES.find((level) => level === named)
    if (!checks.has(id)) checks.set(id, { id, ...(severity === undefined ? {} : { severity }), ...(tree === undefined ? {} : { needsTree: true }) })
  }
  return [...checks.values()]
}

/**
 * A finding whose check declares a severity takes that severity, whatever
 * its critic wrote, and keeps it as `declared_severity`, so record can tell
 * an override from the critic's own rating.
 */
export function withDeclaredSeverity(finding: RunFinding, declared: readonly DeclaredCheck[]): RunFinding {
  const severity = declared.find((check) => check.id === finding.check)?.severity
  return severity === undefined ? finding : { ...finding, declared_severity: severity, severity }
}

/**
 * Whether a finding names a check its critic's prompt declares. Returns a
 * hint when it names none, or one that no lens or role of that prompt
 * declares, and nothing when the check is one of `declared`.
 */
export function checkNamed(finding: Pick<FindingBody, 'check'>, declared: readonly DeclaredCheck[]): PreVerdict | undefined {
  if (finding.check === undefined) return { reason: 'names no check', reason_code: 'no-check', verdict: 'hint' }
  if (declared.some((check) => check.id === finding.check)) return undefined
  return {
    reason: `names the check ${finding.check}, which no lens or role in its critic's prompt declares`,
    reason_code: 'unknown-check',
    verdict: 'hint',
  }
}

/**
 * A finding of a check that needs code under `tree/` cannot be checked in a
 * packet that holds none, so it is a hint: the check was skipped, and the
 * report names it as skipped. A role rule and a lens check are marked alike,
 * and the skipped list reads the same role and lens files.
 */
export function checkRunnable(finding: Pick<FindingBody, 'check'>, declared: readonly DeclaredCheck[], tree: boolean): PreVerdict | undefined {
  if (tree || declared.find((check) => check.id === finding.check)?.needsTree !== true) return undefined
  return NEEDS_TREE
}

/** The hint a finding gets when what it checks needs code under `tree/` and the packet holds none. */
export const NEEDS_TREE: PreVerdict = {
  reason: 'the check needs code under tree/, which this packet does not hold',
  reason_code: 'needs-tree',
  verdict: 'hint',
}
