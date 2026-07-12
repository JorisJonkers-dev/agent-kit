export type MonitorStatus = 'polling' | 'passed' | 'failed' | 'timed-out'

export interface MonitorState {
  readonly name: string
  readonly status: MonitorStatus
  readonly startedAt: string
  readonly deadline: string
  readonly lastTickAt: string
  readonly lastOutput: string
  readonly intervalMs: number
  readonly cmd: string
  readonly until: string
  readonly then: string
}

export function parseDuration(duration: string): number {
  const match = /^(\d+)(s|m|h)$/u.exec(duration)
  if (match === null) throw new Error(`invalid duration: ${duration} (expected e.g. 30s, 45m, 2h)`)
  const value = Number(match[1])
  const unit = match[2]
  if (unit === 's') return value * 1_000
  if (unit === 'm') return value * 60_000
  return value * 3_600_000
}

export function isMonitorDead(state: MonitorState, nowMs: number): boolean {
  if (state.status !== 'polling') return false
  const lastTickMs = new Date(state.lastTickAt).getTime()
  const stalenessThreshold = state.intervalMs * 2.5
  return nowMs - lastTickMs > stalenessThreshold
}

export function evaluatePredicate(predicate: string, output: string): boolean {
  if (predicate.startsWith('.')) {
    return evaluateJsonPath(predicate, output)
  }
  return output.includes(predicate)
}

function evaluateJsonPath(predicate: string, output: string): boolean {
  let parsed: unknown
  try {
    parsed = JSON.parse(output.trim())
  } catch {
    return false
  }

  const eqMatch = /^(.+?)\s*==\s*"([^"]*)"$/u.exec(predicate)
  if (eqMatch !== null) {
    /* c8 ignore next 2 -- regex groups always defined when match succeeds */
    const path = eqMatch[1] ?? ''
    const expected = eqMatch[2] ?? ''
    return resolveJsonPath(parsed, path) === expected
  }

  const value = resolveJsonPath(parsed, predicate)
  return value !== undefined && value !== null && value !== false && value !== ''
}

function resolveJsonPath(value: unknown, path: string): unknown {
  const segments = path
    .split('.')
    .filter((segment) => segment.length > 0)
  let current: unknown = value
  for (const segment of segments) {
    if (current === null || typeof current !== 'object' || Array.isArray(current)) return undefined
    current = (current as Record<string, unknown>)[segment]
  }
  return current
}
