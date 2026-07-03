import type { CodexEffort, CouncilIntensity, CouncilPreset, CouncilRoleKey } from './config-value-objects.js'

export const DEFAULT_INTENSITY: CouncilIntensity = 'standard'
export const ROLE_KEYS = ['planner_a', 'planner_b', 'consolidator', 'worker', 'verifier'] as const
export const INT_KEYS = ['rounds', 'max_workers'] as const
export const CODEX_EFFORTS = ['low', 'medium', 'high', 'xhigh'] as const
export const CONFIG_KEYS = [
  'intensity',
  ...ROLE_KEYS,
  'codex_effort',
  ...INT_KEYS,
] as const

export const BASE_ROLES: Readonly<Record<Exclude<CouncilRoleKey, 'worker'>, string>> = {
  planner_a: 'claude:opus',
  planner_b: 'codex:gpt-5.5',
  consolidator: 'claude:opus',
  verifier: 'claude:sonnet',
}

export const PRESETS: Readonly<Record<CouncilIntensity, CouncilPreset>> = {
  quick: { rounds: 1, codex_effort: 'low', worker: 'claude:haiku', max_workers: 4 },
  standard: { rounds: 2, codex_effort: 'high', worker: 'claude:haiku', max_workers: 6 },
  thorough: { rounds: 3, codex_effort: 'high', worker: 'claude:sonnet', max_workers: 6 },
  max: { rounds: 3, codex_effort: 'xhigh', worker: 'claude:sonnet', max_workers: 8 },
}

export function requireIntensity(value: unknown): CouncilIntensity {
  const parsed = optionalIntensity(value)
  if (parsed === undefined) {
    throw new Error(`unknown intensity ${String(value)}; choose from ${Object.keys(PRESETS).join(', ')}`)
  }
  return parsed
}

export function optionalIntensity(value: unknown): CouncilIntensity | undefined {
  return typeof value === 'string' && value in PRESETS ? (value as CouncilIntensity) : undefined
}

export function requireCodexEffort(value: unknown): CodexEffort {
  const parsed = optionalCodexEffort(value)
  if (parsed === undefined) {
    throw new Error(`codex_effort must be one of ${CODEX_EFFORTS.join(', ')}`)
  }
  return parsed
}

export function optionalCodexEffort(value: unknown): CodexEffort | undefined {
  return typeof value === 'string' && (CODEX_EFFORTS as readonly string[]).includes(value)
    ? (value as CodexEffort)
    : undefined
}
