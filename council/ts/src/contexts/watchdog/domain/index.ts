export interface StallDetectorConfig {
  readonly stallAfterS: number
}

export interface StallDetectorInput extends StallDetectorConfig {
  readonly logBytes: number
  readonly nowMs: number
}

export interface StallDetectorState {
  readonly logBytes: number
  readonly lastGrowthAtMs: number
}

export interface StallDetection {
  readonly kind: 'stall'
  readonly idleMs: number
  readonly logBytes: number
}

export interface StallDetectorResult {
  readonly state: StallDetectorState
  readonly detection: StallDetection | null
}

export interface ActionLine {
  readonly verbatim: string
  readonly normalized: string
}

export interface LoopDetectorConfig {
  readonly windowSize: number
  readonly repeatLimit: number
  readonly maxCycleGram?: number
}

export interface LoopDetectorState {
  readonly actions: readonly ActionLine[]
}

export type LoopDetection =
  | {
      readonly kind: 'loop-repeat'
      readonly normalized: string
      readonly count: number
    }
  | {
      readonly kind: 'loop-cycle'
      readonly gramSize: number
      readonly sequence: readonly string[]
    }

export interface LoopDetectorResult {
  readonly state: LoopDetectorState
  readonly detection: LoopDetection | null
}

export interface DiskUsageCapInput {
  readonly duBytes: number
  readonly capBytes: number
}

export interface DiskUsageCapDetection {
  readonly kind: 'disk-cap'
  readonly duBytes: number
  readonly capBytes: number
}

export type EscalationPhase =
  | 'ready'
  | 'terminated'
  | 'retry-with-preamble'
  | 'tier-escalated'
  | 'stalled'

export type EscalationAction =
  | 'terminate'
  | 'retry-with-preamble'
  | 'escalate-tier'
  | 'stalled'

export interface EscalationState {
  readonly phase: EscalationPhase
}

export interface EscalationConfig {
  readonly enableTierEscalation: boolean
}

export interface EscalationResult {
  readonly state: EscalationState
  readonly action: EscalationAction
}

export function createStallDetectorState(nowMs: number, logBytes = 0): StallDetectorState {
  return { logBytes, lastGrowthAtMs: nowMs }
}

export function evaluateStall(
  state: StallDetectorState,
  input: StallDetectorInput,
): StallDetectorResult {
  if (input.logBytes > state.logBytes) {
    return {
      state: { logBytes: input.logBytes, lastGrowthAtMs: input.nowMs },
      detection: null,
    }
  }

  const idleMs = input.nowMs - state.lastGrowthAtMs
  const stalled = idleMs >= input.stallAfterS * 1000

  return {
    state,
    detection: stalled ? { kind: 'stall', idleMs, logBytes: input.logBytes } : null,
  }
}

export function createLoopDetectorState(): LoopDetectorState {
  return { actions: [] }
}

export function appendLoopLine(
  state: LoopDetectorState,
  line: string,
  config: LoopDetectorConfig,
): LoopDetectorResult {
  const actions = extractActionLines(line)

  if (actions.length === 0) {
    return { state, detection: null }
  }

  const nextActions = [...state.actions, ...actions].slice(-config.windowSize)
  const nextState = { actions: nextActions }

  return {
    state: nextState,
    detection: detectRepeat(nextActions, config) ?? detectCycle(nextActions, config),
  }
}

export function extractActionLines(line: string): readonly ActionLine[] {
  const trimmed = line.trim()

  if (trimmed.length === 0) {
    return []
  }

  const jsonActions = extractJsonActionLines(trimmed)

  if (jsonActions.length > 0) {
    return jsonActions
  }

  const codexCommand = extractCodexCommand(trimmed)

  return codexCommand === null ? [] : [toActionLine(codexCommand)]
}

export function evaluateDiskUsageCap(input: DiskUsageCapInput): DiskUsageCapDetection | null {
  return input.duBytes > input.capBytes
    ? { kind: 'disk-cap', duBytes: input.duBytes, capBytes: input.capBytes }
    : null
}

export function createEscalationState(): EscalationState {
  return { phase: 'ready' }
}

export function advanceEscalation(
  state: EscalationState,
  config: EscalationConfig,
): EscalationResult {
  switch (state.phase) {
    case 'ready':
      return { state: { phase: 'terminated' }, action: 'terminate' }
    case 'terminated':
      return {
        state: { phase: 'retry-with-preamble' },
        action: 'retry-with-preamble',
      }
    case 'retry-with-preamble':
      return config.enableTierEscalation
        ? { state: { phase: 'tier-escalated' }, action: 'escalate-tier' }
        : { state: { phase: 'stalled' }, action: 'stalled' }
    case 'tier-escalated':
    case 'stalled':
      return { state: { phase: 'stalled' }, action: 'stalled' }
  }
}

function detectRepeat(
  actions: readonly ActionLine[],
  config: LoopDetectorConfig,
): LoopDetection | null {
  const counts = new Map<string, number>()

  for (const action of actions) {
    const count = (counts.get(action.normalized) ?? 0) + 1

    if (count >= config.repeatLimit) {
      return { kind: 'loop-repeat', normalized: action.normalized, count }
    }

    counts.set(action.normalized, count)
  }

  return null
}

function detectCycle(
  actions: readonly ActionLine[],
  config: LoopDetectorConfig,
): LoopDetection | null {
  const maxGram = Math.min(config.maxCycleGram ?? 5, 5, Math.floor(actions.length / 3))

  for (let gramSize = 1; gramSize <= maxGram; gramSize += 1) {
    const offset = actions.length - gramSize
    const sequence = actions.slice(offset).map((action) => action.verbatim)

    if (matchesCycle(actions, sequence, gramSize)) {
      return { kind: 'loop-cycle', gramSize, sequence }
    }
  }

  return null
}

function matchesCycle(
  actions: readonly ActionLine[],
  sequence: readonly string[],
  gramSize: number,
): boolean {
  for (let repeat = 2; repeat <= 3; repeat += 1) {
    const offset = actions.length - repeat * gramSize

    for (let index = 0; index < gramSize; index += 1) {
      if (actions[offset + index]?.verbatim !== sequence[index]) {
        return false
      }
    }
  }

  return true
}

function extractJsonActionLines(line: string): readonly ActionLine[] {
  try {
    const parsed: unknown = JSON.parse(line)

    return findToolUses(parsed)
  } catch {
    return []
  }
}

function findToolUses(value: unknown): readonly ActionLine[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => findToolUses(item))
  }

  if (!isRecord(value)) {
    return []
  }

  const ownToolUse = value.type === 'tool_use' ? [toolUseToActionLine(value)] : []
  const nestedToolUses = Object.values(value).flatMap((item) => findToolUses(item))

  return [...ownToolUse, ...nestedToolUses]
}

function toolUseToActionLine(toolUse: Record<string, unknown>): ActionLine {
  const name = typeof toolUse.name === 'string' ? toolUse.name : 'unknown'
  const input = isRecord(toolUse.input) ? toolUse.input : {}

  return toActionLine(`tool:${name}:${stableStringify(input)}`)
}

function extractCodexCommand(line: string): string | null {
  const match = /^(?:[$>]|exec(?:_command)?[: ]|command[: ])\s*(?<command>.+)$/iu.exec(line)

  return match?.groups?.command?.trim() ?? null
}

function toActionLine(verbatim: string): ActionLine {
  return {
    verbatim,
    normalized: normalizeActionLine(verbatim),
  }
}

function normalizeActionLine(line: string): string {
  return line
    .trim()
    .toLowerCase()
    .replace(/[0-9a-f]{8,}/giu, '<hex>')
    .replace(/\d+/gu, '<n>')
    .replace(/\s+/gu, ' ')
}

function stableStringify(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(',')}]`
  }

  if (isRecord(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(',')}}`
  }

  const serialized: unknown = JSON.stringify(value)
  return typeof serialized === 'string' ? serialized : 'undefined'
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
