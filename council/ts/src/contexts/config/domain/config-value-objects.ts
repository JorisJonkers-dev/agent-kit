export type TomlPrimitive = string | number | boolean
export type TomlValue = TomlPrimitive | readonly TomlValue[] | TomlTable | readonly TomlTable[]

export interface TomlTable {
  readonly [key: string]: TomlValue
}

export type CouncilIntensity = 'quick' | 'standard' | 'thorough' | 'max'
export type CodexEffort = 'low' | 'medium' | 'high' | 'xhigh'
export type CouncilRoleKey = 'planner_a' | 'planner_b' | 'consolidator' | 'worker' | 'verifier'

export interface EngineCommandConfig {
  readonly argv?: readonly string[]
  readonly stream_format?: string
  readonly result_extraction?: string
}

export interface StageEngineConfig {
  readonly engine?: string
  readonly effort?: string
}

export interface CouncilConfig {
  readonly intensity?: CouncilIntensity
  readonly planner_a?: string
  readonly planner_b?: string
  readonly consolidator?: string
  readonly worker?: string
  readonly verifier?: string
  readonly codex_effort?: CodexEffort
  readonly rounds?: number
  readonly max_workers?: number
  readonly watchdog?: {
    readonly stall_after_s?: number
    readonly window?: number
    readonly repeat_limit?: number
    readonly max_restarts?: number
    readonly escalate_model?: string
    readonly disk_cap_gib?: number
  }
  readonly design?: {
    readonly lenses?: readonly string[]
    readonly rounds?: number
    readonly stages?: Readonly<Record<string, StageEngineConfig>>
  }
  readonly review?: {
    readonly council?: boolean
    readonly max_fix_rounds?: number
    readonly difficulty?: Readonly<Record<string, string>>
  }
  readonly github?: {
    readonly enabled?: boolean
    readonly assignee?: string
  }
  readonly engines?: Readonly<Record<string, EngineCommandConfig>>
  readonly triage?: {
    readonly matrix_overrides?: Readonly<Record<string, string>>
  }
  readonly context?: {
    readonly pack_stale_after_s?: number
  }
  readonly model_matrix?: {
    readonly roles?: Partial<Record<CouncilRoleKey, string>>
    readonly intensity?: Partial<Record<CouncilIntensity, Partial<CouncilPreset>>>
  }
}

export interface CouncilPreset {
  readonly rounds: number
  readonly codex_effort: CodexEffort
  readonly worker: string
  readonly max_workers: number
}

export interface CouncilRuntimeConfig {
  readonly codex_reasoning: string
  readonly plan_timeout_s: number
  readonly worker_timeout_s: number
  readonly verify_timeout_s: number
}

export interface ResolvedCouncilConfig extends CouncilConfig {
  readonly intensity: CouncilIntensity
  readonly planner_a: string
  readonly planner_b: string
  readonly consolidator: string
  readonly worker: string
  readonly verifier: string
  readonly codex_effort: CodexEffort
  readonly rounds: number
  readonly max_workers: number
  readonly runtime: CouncilRuntimeConfig
}

export interface TomlAssignment {
  readonly lineIndex: number
  readonly tablePath: readonly string[]
  readonly keyPath: readonly string[]
  readonly sourceKey: string
}

export interface TomlTableHeader {
  readonly lineIndex: number
  readonly path: readonly string[]
  readonly array: boolean
}

export interface TomlDocument {
  readonly source: string
  readonly lines: readonly string[]
  readonly finalNewline: boolean
  readonly data: TomlTable
  readonly assignments: readonly TomlAssignment[]
  readonly tables: readonly TomlTableHeader[]
}

export function requireString(value: unknown, key: string): string {
  const parsed = optionalString(value)
  if (parsed === undefined) {
    throw new Error(`${key} must be a string`)
  }
  return parsed
}

export function requireNumber(value: unknown, key: string): number {
  const parsed = optionalNumber(value)
  if (parsed === undefined) {
    throw new Error(`${key} must be a number`)
  }
  return parsed
}

export function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

export function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined
}

export function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

export function optionalStringArray(value: unknown): readonly string[] | undefined {
  return Array.isArray(value) && value.every((item) => typeof item === 'string') ? value : undefined
}

export function optionalObject<T extends object>(
  value: unknown,
  map: (table: Readonly<Record<string, TomlValue>>) => T,
): T | undefined {
  const table = asTomlTable(value)
  if (!table) {
    return undefined
  }
  const mapped = map(table)
  return Object.keys(mapped).length > 0 ? mapped : undefined
}

export function optionalStringRecord(value: unknown): Readonly<Record<string, string>> | undefined {
  const table = asTomlTable(value)
  if (!table) {
    return undefined
  }
  const entries = Object.entries(table).filter(
    (entry): entry is [string, string] => typeof entry[1] === 'string',
  )
  return entries.length > 0 ? Object.fromEntries(entries) : undefined
}

export function optionalStringRecordTable<T extends object>(
  value: unknown,
  map: (table: Readonly<Record<string, TomlValue>>) => T,
): Readonly<Record<string, T>> | undefined {
  const table = asTomlTable(value)
  if (!table) {
    return undefined
  }
  const entries = Object.entries(table)
    .map(([key, entry]) => [key, latestTable(entry)] as const)
    .filter((entry): entry is readonly [string, TomlTable] => entry[1] !== undefined)
    .map(([key, table]) => [key, map(table)] as [string, T])
    .filter((entry): entry is [string, T] => Object.keys(entry[1]).length > 0)
  return entries.length > 0 ? Object.fromEntries(entries) : undefined
}

export function asTomlTable(value: unknown): TomlTable | undefined {
  return isTomlTable(value) ? value : undefined
}

export function isTomlTable(value: unknown): value is TomlTable {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function omitUndefined<T extends object>(value: T): {
  [K in keyof T]?: Exclude<T[K], undefined>
} {
  return Object.fromEntries(Object.entries(value).filter((entry) => entry[1] !== undefined)) as {
    [K in keyof T]?: Exclude<T[K], undefined>
  }
}

function latestTable(value: unknown): TomlTable | undefined {
  const table = asTomlTable(value)
  if (table) {
    return table
  }
  if (Array.isArray(value)) {
    return value
      .map((item) => asTomlTable(item))
      .filter((item): item is TomlTable => item !== undefined)
      .at(-1)
  }
  return undefined
}
