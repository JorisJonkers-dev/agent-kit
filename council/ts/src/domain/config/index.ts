import type { EnvPort } from '../../ports/index.js'

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

export interface ResolveCouncilConfigInput {
  readonly preset?: CouncilIntensity
  readonly user?: CouncilConfig
  readonly project?: CouncilConfig
  readonly flags?: CouncilConfig
  readonly env?: EnvPort
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

const ENV_DEFAULTS = {
  COUNCIL_CODEX_REASONING: 'high',
  COUNCIL_PLAN_TIMEOUT_S: 1200,
  COUNCIL_WORKER_TIMEOUT_S: 1800,
  COUNCIL_VERIFY_TIMEOUT_S: 600,
} as const

export function parseToml(source: string): TomlDocument {
  const finalNewline = source.endsWith('\n')
  const lines = source.replace(/\r\n/g, '\n').split('\n')
  if (finalNewline) {
    lines.pop()
  }
  const data: MutableTomlTable = {}
  const assignments: TomlAssignment[] = []
  const tables: TomlTableHeader[] = []
  let currentPath: readonly string[] = []
  let currentTable: MutableTomlTable = data

  lines.forEach((line, lineIndex) => {
    const body = stripInlineComment(line).trim()
    if (body === '') {
      return
    }
    const arrayHeader = /^\[\[(.+)\]\]$/.exec(body)
    const tableHeader = /^\[(.+)\]$/.exec(body)
    if (arrayHeader) {
      currentPath = parseKeyPath(arrayHeader[1] ?? '')
      currentTable = appendArrayTable(data, currentPath)
      tables.push({ lineIndex, path: currentPath, array: true })
      return
    }
    if (tableHeader) {
      currentPath = parseKeyPath(tableHeader[1] ?? '')
      currentTable = ensureTable(data, currentPath)
      tables.push({ lineIndex, path: currentPath, array: false })
      return
    }
    const equalIndex = findTopLevelChar(body, '=')
    if (equalIndex < 1) {
      throw new Error(`invalid TOML assignment on line ${String(lineIndex + 1)}`)
    }
    const rawKey = body.slice(0, equalIndex).trim()
    const keyPath = parseKeyPath(rawKey)
    const value = parseTomlValue(body.slice(equalIndex + 1).trim())
    setNestedValue(currentTable, keyPath, value)
    assignments.push({ lineIndex, tablePath: currentPath, keyPath, sourceKey: rawKey })
  })

  return { source, lines, finalNewline, data, assignments, tables }
}

export function parseCouncilConfig(source: string): CouncilConfig {
  return normalizeCouncilConfig(parseToml(source).data)
}

export function writeTomlValue(value: TomlValue): string {
  if (typeof value === 'string') {
    return JSON.stringify(value)
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new Error(`TOML number must be finite, got ${String(value)}`)
    }
    return String(value)
  }
  if (typeof value === 'boolean') {
    return value ? 'true' : 'false'
  }
  if (isTomlArray(value)) {
    return `[${value.map((item) => writeTomlValue(item)).join(', ')}]`
  }
  throw new Error('inline TOML tables are not supported by the council writer')
}

function isTomlArray(value: TomlValue): value is readonly TomlValue[] {
  return Array.isArray(value)
}

export function writeCouncilConfig(source: string, config: CouncilConfig): string {
  return writeTomlUpdates(parseToml(source), flattenCouncilConfig(config))
}

export function writeTomlUpdates(
  document: TomlDocument,
  updates: ReadonlyMap<string, TomlValue>,
): string {
  const lines = [...document.lines]
  const written = new Set<string>()
  const assignments = [...document.assignments].sort((a, b) => b.lineIndex - a.lineIndex)

  assignments.forEach((assignment) => {
    const updateKey = pathKey([...assignment.tablePath, ...assignment.keyPath])
    const value = updates.get(updateKey)
    if (value === undefined || written.has(updateKey)) {
      return
    }
    lines[assignment.lineIndex] = replaceAssignmentValue(
      lines[assignment.lineIndex] ?? '',
      assignment.sourceKey,
      value,
    )
    written.add(updateKey)
  })

  const existingInserts = new Map<number, string[]>()
  const newTableInserts = new Map<string, { path: readonly string[]; lines: string[] }>()
  updates.forEach((value, updateKey) => {
    if (written.has(updateKey)) {
      return
    }
    queueMissingAssignment(document, existingInserts, newTableInserts, updateKey.split('.'), value)
  })

  const orderedExistingInserts = [...existingInserts.entries()].sort(
    ([left], [right]) => right - left,
  )
  orderedExistingInserts.forEach(([index, insertLines]) => lines.splice(index, 0, ...insertLines))

  newTableInserts.forEach((insert) => {
    if (lines.length > 0 && lines.at(-1) !== '') {
      lines.push('')
    }
    lines.push(`[${insert.path.join('.')}]`, ...insert.lines)
  })

  return `${lines.join('\n')}\n`
}

export function resolveCouncilConfig(input: ResolveCouncilConfigInput = {}): ResolvedCouncilConfig {
  const mergedBeforeFlags = mergeCouncilConfigs(input.user, input.project)
  const intensity = resolveIntensity(input.preset, mergedBeforeFlags, input.flags)
  const preset = PRESETS[intensity]
  const resolved = mergeCouncilConfigs(
    { ...BASE_ROLES, ...preset },
    input.user,
    input.project,
    input.flags,
    { intensity },
  )

  return {
    ...resolved,
    intensity,
    planner_a: requireString(resolved.planner_a, 'planner_a'),
    planner_b: requireString(resolved.planner_b, 'planner_b'),
    consolidator: requireString(resolved.consolidator, 'consolidator'),
    worker: requireString(resolved.worker, 'worker'),
    verifier: requireString(resolved.verifier, 'verifier'),
    codex_effort: requireCodexEffort(resolved.codex_effort),
    rounds: requireNumber(resolved.rounds, 'rounds'),
    max_workers: requireNumber(resolved.max_workers, 'max_workers'),
    runtime: resolveRuntime(input.env, resolved.codex_effort),
  }
}

export function coerceConfigValue(key: string, raw: string): string | number {
  if (!isConfigKey(key)) {
    throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(', ')}`)
  }
  switch (key) {
    case 'intensity':
      return requireIntensity(raw)
    case 'rounds':
    case 'max_workers': {
      const parsed = Number.parseInt(raw, 10)
      if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
        throw new Error(`${key} must be an integer, got ${raw}`)
      }
      return parsed
    }
    case 'codex_effort':
      return requireCodexEffort(raw)
    case 'planner_a':
    case 'planner_b':
    case 'consolidator':
    case 'worker':
    case 'verifier':
      if (!/^(claude|codex):.+/.test(raw)) {
        throw new Error(`${key} must be claude:<model> or codex:<model>, got ${raw}`)
      }
      return raw
    }
}

type MutableTomlTable = Record<string, TomlValue>;

function parseTomlValue(raw: string): TomlValue {
  if (raw.startsWith('"')) {
    return parseString(raw)
  }
  if (raw.startsWith('[')) {
    return parseArray(raw)
  }
  if (raw === 'true') {
    return true
  }
  if (raw === 'false') {
    return false
  }
  if (/^[+-]?\d+$/.test(raw)) {
    return Number.parseInt(raw, 10)
  }
  throw new Error(`unsupported TOML value ${raw}`)
}

function parseString(raw: string): string {
  const commentIndex = findValueEnd(raw)
  const candidate = raw.slice(0, commentIndex)
  try {
    return JSON.parse(candidate) as string
  } catch {
    throw new Error(`invalid TOML string ${raw}`)
  }
}

function parseArray(raw: string): readonly TomlValue[] {
  const end = findMatchingBracket(raw)
  if (end < 0) {
    throw new Error(`unterminated TOML array ${raw}`)
  }
  const inner = raw.slice(1, end).trim()
  if (inner === '') {
    return []
  }
  return splitTopLevel(inner, ',').map((part) => parseTomlValue(part.trim()))
}

function parseKeyPath(raw: string): readonly string[] {
  const parts = splitTopLevel(raw.trim(), '.').map((part) => part.trim())
  if (parts.some((part) => !/^[A-Za-z0-9_-]+$/.test(part))) {
    throw new Error(`unsupported TOML key path ${raw}`)
  }
  return parts
}

function stripInlineComment(line: string): string {
  const commentIndex = findCommentIndex(line)
  return commentIndex < 0 ? line : line.slice(0, commentIndex)
}

function findCommentIndex(line: string): number {
  let inString = false
  let escaped = false
  let depth = 0
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index]
    if (escaped) {
      escaped = false
      continue
    }
    if (char === '\\' && inString) {
      escaped = true
      continue
    }
    if (char === '"') {
      inString = !inString
      continue
    }
    if (!inString && char === '[') {
      depth += 1
      continue
    }
    if (!inString && char === ']') {
      depth -= 1
      continue
    }
    if (!inString && depth === 0 && char === '#') {
      return index
    }
  }
  return -1
}

function findTopLevelChar(line: string, target: string): number {
  let inString = false
  let escaped = false
  let depth = 0
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index]
    if (escaped) {
      escaped = false
      continue
    }
    if (char === '\\' && inString) {
      escaped = true
      continue
    }
    if (char === '"') {
      inString = !inString
      continue
    }
    if (!inString && char === '[') {
      depth += 1
      continue
    }
    if (!inString && char === ']') {
      depth -= 1
      continue
    }
    if (!inString && depth === 0 && char === target) {
      return index
    }
  }
  return -1
}

function splitTopLevel(raw: string, delimiter: string): string[] {
  const parts: string[] = []
  let start = 0
  let inString = false
  let escaped = false
  let depth = 0
  for (let index = 0; index < raw.length; index += 1) {
    const char = raw[index]
    if (escaped) {
      escaped = false
      continue
    }
    if (char === '\\' && inString) {
      escaped = true
      continue
    }
    if (char === '"') {
      inString = !inString
      continue
    }
    if (!inString && char === '[') {
      depth += 1
      continue
    }
    if (!inString && char === ']') {
      depth -= 1
      continue
    }
    if (!inString && depth === 0 && char === delimiter) {
      parts.push(raw.slice(start, index))
      start = index + 1
    }
  }
  parts.push(raw.slice(start))
  return parts
}

function findMatchingBracket(raw: string): number {
  let inString = false
  let escaped = false
  let depth = 0
  for (let index = 0; index < raw.length; index += 1) {
    const char = raw[index]
    if (escaped) {
      escaped = false
      continue
    }
    if (char === '\\' && inString) {
      escaped = true
      continue
    }
    if (char === '"') {
      inString = !inString
      continue
    }
    if (!inString && char === '[') {
      depth += 1
      continue
    }
    if (!inString && char === ']') {
      depth -= 1
      if (depth === 0) {
        return index
      }
    }
  }
  return -1
}

function findValueEnd(raw: string): number {
  const commentIndex = findCommentIndex(raw)
  return commentIndex < 0 ? raw.length : commentIndex
}

function setNestedValue(table: MutableTomlTable, keyPath: readonly string[], value: TomlValue): void {
  const head = keyPath[0] ?? ''
  const tail = keyPath.slice(1)
  if (tail.length === 0) {
    table[head] = value
    return
  }
  const child = table[head]
  if (!isTomlTable(child)) {
    table[head] = {}
  }
  setNestedValue(table[head] as MutableTomlTable, tail, value)
}

function ensureTable(root: MutableTomlTable, path: readonly string[]): MutableTomlTable {
  let table = root
  path.forEach((part) => {
    const next = table[part]
    if (Array.isArray(next)) {
      throw new Error(`TOML table conflicts with existing array table ${path.join('.')}`)
    }
    if (!isTomlTable(next)) {
      table[part] = {}
    }
    table = table[part] as MutableTomlTable
  })
  return table
}

function appendArrayTable(root: MutableTomlTable, path: readonly string[]): MutableTomlTable {
  const parent = ensureTable(root, path.slice(0, -1))
  const name = path.at(-1) ?? ''
  const current = parent[name]
  const next: MutableTomlTable = {}
  if (current === undefined) {
    parent[name] = [next]
    return next
  }
  if (Array.isArray(current)) {
    const tables = current as MutableTomlTable[]
    tables.push(next)
    return next
  }
  throw new Error(`TOML array table conflicts with existing table ${path.join('.')}`)
}

function isTomlTable(value: unknown): value is TomlTable {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function replaceAssignmentValue(line: string, sourceKey: string, value: TomlValue): string {
  const commentIndex = findCommentIndex(line)
  const suffix = commentIndex < 0 ? '' : line.slice(commentIndex)
  const prefixMatch = /^(\s*)/.exec(line)
  const prefix = prefixMatch?.[1] ?? ''
  const spacing = suffix === '' ? '' : ' '
  return `${prefix}${sourceKey} = ${writeTomlValue(value)}${spacing}${suffix}`.trimEnd()
}

function queueMissingAssignment(
  document: TomlDocument,
  existingInserts: Map<number, string[]>,
  newTableInserts: Map<string, { path: readonly string[]; lines: string[] }>,
  fullPath: readonly string[],
  value: TomlValue,
): void {
  const key = fullPath.at(-1)
  if (!key) {
    throw new Error('cannot write empty TOML path')
  }
  const tablePath = fullPath.slice(0, -1)
  const insertLine = `${key} = ${writeTomlValue(value)}`
  const table = findTable(document, tablePath)
  if (!table && tablePath.length > 0) {
    const tableKey = pathKey(tablePath)
    const pending = newTableInserts.get(tableKey)
    if (pending) {
      pending.lines.push(insertLine)
    } else {
      newTableInserts.set(tableKey, { path: tablePath, lines: [insertLine] })
    }
    return
  }
  const index = table ? findTableInsertIndex(document, table) : findRootInsertIndex(document)
  const pending = existingInserts.get(index)
  if (pending) {
    pending.push(insertLine)
  } else {
    existingInserts.set(index, [insertLine])
  }
}

function findTable(document: TomlDocument, tablePath: readonly string[]): TomlTableHeader | undefined {
  const tables = [...document.tables].reverse()
  return (
    tables.find((table) => !table.array && samePath(table.path, tablePath)) ??
    tables.find((table) => table.array && samePath(table.path, tablePath))
  )
}

function findTableInsertIndex(document: TomlDocument, table: TomlTableHeader): number {
  const next = document.tables.find((candidate) => candidate.lineIndex > table.lineIndex)
  return next?.lineIndex ?? document.lines.length
}

function findRootInsertIndex(document: TomlDocument): number {
  return document.tables[0]?.lineIndex ?? document.lines.length
}

function samePath(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((part, index) => part === right[index])
}

function pathKey(path: readonly string[]): string {
  return path.join('.')
}

function normalizeCouncilConfig(data: TomlTable): CouncilConfig {
  return omitUndefined({
    intensity: optionalIntensity(data.intensity),
    planner_a: optionalString(data.planner_a),
    planner_b: optionalString(data.planner_b),
    consolidator: optionalString(data.consolidator),
    worker: optionalString(data.worker),
    verifier: optionalString(data.verifier),
    codex_effort: optionalCodexEffort(data.codex_effort),
    rounds: optionalNumber(data.rounds),
    max_workers: optionalNumber(data.max_workers),
    watchdog: optionalObject(data.watchdog, (watchdog) =>
      omitUndefined({
        stall_after_s: optionalNumber(watchdog.stall_after_s),
        window: optionalNumber(watchdog.window),
        repeat_limit: optionalNumber(watchdog.repeat_limit),
        max_restarts: optionalNumber(watchdog.max_restarts),
        escalate_model: optionalString(watchdog.escalate_model),
        disk_cap_gib: optionalNumber(watchdog.disk_cap_gib),
      }),
    ),
    design: optionalObject(data.design, (design) =>
      omitUndefined({
        lenses: optionalStringArray(design.lenses),
        rounds: optionalNumber(design.rounds),
        stages: optionalStringRecordTable(design.stages, (stage) =>
          omitUndefined({
            engine: optionalString(stage.engine),
            effort: optionalString(stage.effort),
          }),
        ),
      }),
    ),
    review: optionalObject(data.review, (review) =>
      omitUndefined({
        council: optionalBoolean(review.council),
        max_fix_rounds: optionalNumber(review.max_fix_rounds),
        difficulty: optionalStringRecord(review.difficulty),
      }),
    ),
    github: optionalObject(data.github, (github) =>
      omitUndefined({
        enabled: optionalBoolean(github.enabled),
        assignee: optionalString(github.assignee),
      }),
    ),
    engines: optionalStringRecordTable(data.engines, (engine) =>
      omitUndefined({
        argv: optionalStringArray(engine.argv),
        stream_format: optionalString(engine.stream_format),
        result_extraction: optionalString(engine.result_extraction),
      }),
    ),
    triage: optionalObject(data.triage, (triage) =>
      omitUndefined({
        matrix_overrides: optionalStringRecord(triage.matrix_overrides),
      }),
    ),
    context: optionalObject(data.context, (context) =>
      omitUndefined({
        pack_stale_after_s: optionalNumber(context.pack_stale_after_s),
      }),
    ),
    model_matrix: optionalObject(data.model_matrix, (modelMatrix) =>
      omitUndefined({
        roles: optionalRoleRecord(modelMatrix.roles),
        intensity: optionalStringRecordTable(modelMatrix.intensity, (preset) =>
          omitUndefined({
            rounds: optionalNumber(preset.rounds),
            codex_effort: optionalCodexEffort(preset.codex_effort),
            worker: optionalString(preset.worker),
            max_workers: optionalNumber(preset.max_workers),
          }) as Partial<CouncilPreset>,
        ),
      }),
    ),
  })
}

function flattenCouncilConfig(config: CouncilConfig): Map<string, TomlValue> {
  const updates = new Map<string, TomlValue>()
  addScalars(updates, [], config, CONFIG_KEYS)
  addScalars(updates, ['watchdog'], config.watchdog, [
    'stall_after_s',
    'window',
    'repeat_limit',
    'max_restarts',
    'escalate_model',
    'disk_cap_gib',
  ])
  addScalars(updates, ['design'], config.design, ['lenses', 'rounds'])
  addNestedScalars(updates, ['design', 'stages'], config.design?.stages, ['engine', 'effort'])
  addScalars(updates, ['review'], config.review, ['council', 'max_fix_rounds'])
  addRecord(updates, ['review', 'difficulty'], config.review?.difficulty)
  addScalars(updates, ['github'], config.github, ['enabled', 'assignee'])
  addNestedScalars(updates, ['engines'], config.engines, ['argv', 'stream_format', 'result_extraction'])
  addRecord(updates, ['triage', 'matrix_overrides'], config.triage?.matrix_overrides)
  addScalars(updates, ['context'], config.context, ['pack_stale_after_s'])
  addRecord(updates, ['model_matrix', 'roles'], config.model_matrix?.roles)
  addNestedScalars(updates, ['model_matrix', 'intensity'], config.model_matrix?.intensity, [
    'rounds',
    'codex_effort',
    'worker',
    'max_workers',
  ])
  return updates
}

function addScalars(
  updates: Map<string, TomlValue>,
  prefix: readonly string[],
  source: object | undefined,
  keys: readonly string[],
): void {
  if (!source) {
    return
  }
  keys.forEach((key) => {
    const value = (source as Readonly<Record<string, TomlValue | undefined>>)[key]
    if (value !== undefined) {
      updates.set(pathKey([...prefix, key]), value)
    }
  })
}

function addRecord(
  updates: Map<string, TomlValue>,
  prefix: readonly string[],
  source: object | undefined,
): void {
  if (!source) {
    return
  }
  Object.entries(source).forEach(([key, value]) => {
    if (value !== undefined) {
      updates.set(pathKey([...prefix, key]), value as TomlValue)
    }
  })
}

function addNestedScalars(
  updates: Map<string, TomlValue>,
  prefix: readonly string[],
  source: Readonly<Record<string, object | undefined>> | undefined,
  keys: readonly string[],
): void {
  if (!source) {
    return
  }
  Object.entries(source).forEach(([name, value]) => { addScalars(updates, [...prefix, name], value, keys); })
}

function mergeCouncilConfigs(...configs: readonly (CouncilConfig | undefined)[]): CouncilConfig {
  return configs.reduce<CouncilConfig>((merged, config) => deepMerge(merged, config), {})
}

function deepMerge<T extends object>(left: T, right: T | undefined): T {
  if (!right) {
    return left
  }
  const merged: Record<string, unknown> = { ...(left as Record<string, unknown>) }
  Object.entries(right).forEach(([key, value]) => {
    if (value === undefined) {
      return
    }
    const existing = merged[key]
    merged[key] = isPlainObject(existing) && isPlainObject(value) ? deepMerge(existing, value) : value
  })
  return merged as T
}

function resolveIntensity(
  preset: CouncilIntensity | undefined,
  config: CouncilConfig,
  flags: CouncilConfig | undefined,
): CouncilIntensity {
  return requireIntensity(flags?.intensity ?? config.intensity ?? preset ?? DEFAULT_INTENSITY)
}

function resolveRuntime(env: EnvPort | undefined, codexEffort: CodexEffort | undefined): CouncilRuntimeConfig {
  const codexReasoning = env?.get('COUNCIL_CODEX_REASONING') ?? codexEffort ?? ENV_DEFAULTS.COUNCIL_CODEX_REASONING
  return {
    codex_reasoning: codexReasoning,
    plan_timeout_s: envInt(env, 'COUNCIL_PLAN_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_PLAN_TIMEOUT_S),
    worker_timeout_s: envInt(env, 'COUNCIL_WORKER_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_WORKER_TIMEOUT_S),
    verify_timeout_s: envInt(env, 'COUNCIL_VERIFY_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_VERIFY_TIMEOUT_S),
  }
}

function envInt(env: EnvPort | undefined, name: string, fallback: number): number {
  const raw = env?.get(name)
  if (raw === undefined) {
    return fallback
  }
  const parsed = Number.parseInt(raw, 10)
  if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
    throw new Error(`${name} must be an integer, got ${raw}`)
  }
  return parsed
}

function requireString(value: unknown, key: string): string {
  const parsed = optionalString(value)
  if (parsed === undefined) {
    throw new Error(`${key} must be a string`)
  }
  return parsed
}

function requireNumber(value: unknown, key: string): number {
  const parsed = optionalNumber(value)
  if (parsed === undefined) {
    throw new Error(`${key} must be a number`)
  }
  return parsed
}

function requireIntensity(value: unknown): CouncilIntensity {
  const parsed = optionalIntensity(value)
  if (parsed === undefined) {
    throw new Error(`unknown intensity ${String(value)}; choose from ${Object.keys(PRESETS).join(', ')}`)
  }
  return parsed
}

function requireCodexEffort(value: unknown): CodexEffort {
  const parsed = optionalCodexEffort(value)
  if (parsed === undefined) {
    throw new Error(`codex_effort must be one of ${CODEX_EFFORTS.join(', ')}`)
  }
  return parsed
}

function optionalIntensity(value: unknown): CouncilIntensity | undefined {
  return typeof value === 'string' && value in PRESETS ? (value as CouncilIntensity) : undefined
}

function optionalCodexEffort(value: unknown): CodexEffort | undefined {
  return typeof value === 'string' && (CODEX_EFFORTS as readonly string[]).includes(value)
    ? (value as CodexEffort)
    : undefined
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

function optionalStringArray(value: unknown): readonly string[] | undefined {
  return Array.isArray(value) && value.every((item) => typeof item === 'string') ? value : undefined
}

function optionalObject<T extends object>(
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

function optionalStringRecord(value: unknown): Readonly<Record<string, string>> | undefined {
  const table = asTomlTable(value)
  if (!table) {
    return undefined
  }
  const entries = Object.entries(table).filter((entry): entry is [string, string] => typeof entry[1] === 'string')
  return entries.length > 0 ? Object.fromEntries(entries) : undefined
}

function optionalRoleRecord(value: unknown): Partial<Record<CouncilRoleKey, string>> | undefined {
  const record = optionalStringRecord(value)
  if (!record) {
    return undefined
  }
  return Object.fromEntries(Object.entries(record).filter(([key]) => (ROLE_KEYS as readonly string[]).includes(key)))
}

function optionalStringRecordTable<T extends object>(
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

function asTomlTable(value: unknown): TomlTable | undefined {
  return isTomlTable(value) ? value : undefined
}

function omitUndefined<T extends object>(value: T): { [K in keyof T]?: Exclude<T[K], undefined> } {
  return Object.fromEntries(Object.entries(value).filter((entry) => entry[1] !== undefined)) as {
    [K in keyof T]?: Exclude<T[K], undefined>
  }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isConfigKey(key: string): key is (typeof CONFIG_KEYS)[number] {
  return (CONFIG_KEYS as readonly string[]).includes(key)
}
